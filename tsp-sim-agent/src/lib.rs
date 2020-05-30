extern crate rand;
extern crate serde;

use rand::prelude::{thread_rng, Rng, SliceRandom, ThreadRng};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Location {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

impl Location {
    pub fn distance(&self, other: &Location) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        ((dx * dx) + (dy * dy)).sqrt()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Route {
    pub locations: Vec<Location>,
    pub distance: f64,
}

impl Route {
    fn new(locations: Vec<Location>) -> Route {
        let distance = locations_distance(&locations);
        Route {
            locations,
            distance,
        }
    }

    fn randomized<R>(mut locations: Vec<Location>, rng: &mut R) -> Route
    where
        R: Rng + ?Sized,
    {
        locations.shuffle(rng);
        Route::new(locations)
    }
}

fn locations_distance(locations: &[Location]) -> f64 {
    locations
        .windows(2)
        .fold(0f64, |acc, window| match &window {
            &[loc_a, loc_b] => acc + loc_a.distance(&loc_b),
            _ => acc,
        })
}

// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Simulation {
    pub locations: Vec<Location>,
    pub population_size: usize,
    pub max_iterations: Option<usize>,
    pub assume_convergence: Option<usize>,
}

#[derive(Debug)]
pub enum SimulationEvent {
    Started,
    Iteration(usize),
    NewChampion(Route, usize),
    Finished,
}

impl Simulation {
    const MATING_POOL_SIZE: usize = 7;

    pub fn new(locations: Vec<Location>) -> Simulation {
        Simulation {
            locations,
            population_size: 200,
            max_iterations: Some(100_000),
            assume_convergence: Some(25_000),
        }
    }

    pub fn run<F>(&self, stop: &Arc<AtomicBool>, simulation_event_callback: F) -> Route
    where
        F: Fn(SimulationEvent) -> (),
    {
        assert!(self.population_size > Simulation::MATING_POOL_SIZE);
        assert!(
            self.max_iterations.is_none()
                || self.assume_convergence.is_none()
                || self.max_iterations.unwrap() > self.assume_convergence.unwrap()
        );

        simulation_event_callback(SimulationEvent::Started);

        if self.locations.len() <= 2 {
            let champion = Route::new(self.locations.clone());
            simulation_event_callback(SimulationEvent::NewChampion(champion.to_owned(), 0));
            return champion;
        }

        let mut rng = thread_rng();

        let mut population = self.initial_random_population(&mut rng);
        let mut mating_pool = Simulation::allocate_mating_pool(&population);
        Simulation::select_mating_pool(&population, &mut mating_pool);

        let mut champion = mating_pool[0].to_owned();
        let mut champion_iterations: usize = 0;
        simulation_event_callback(SimulationEvent::NewChampion(champion.to_owned(), 0));

        let max_iterations = self.max_iterations.unwrap_or(usize::MAX);
        let assume_convergence = self.assume_convergence.unwrap_or(usize::MAX);
        let mut iteration: usize = 0;
        loop {
            iteration += 1;
            champion_iterations += 1;
            self.next_generation(&mut population, &mating_pool, &mut rng);
            Simulation::select_mating_pool(&population, &mut mating_pool);
            if champion.distance > mating_pool[0].distance {
                champion = mating_pool[0].to_owned();
                champion_iterations = 0;
                simulation_event_callback(SimulationEvent::NewChampion(
                    champion.to_owned(),
                    iteration,
                ));
            }
            if iteration % 1000 == 0 {
                simulation_event_callback(SimulationEvent::Iteration(iteration));
            }
            if stop.load(Ordering::Relaxed)
                || (self.max_iterations.is_some() && iteration >= max_iterations)
                || (self.assume_convergence.is_some() && champion_iterations >= assume_convergence)
            {
                break;
            }
        }

        simulation_event_callback(SimulationEvent::Finished);
        champion
    }

    fn initial_random_population(&self, rng: &mut ThreadRng) -> Vec<Route> {
        let mut population = Vec::<Route>::with_capacity(self.population_size);
        population.resize_with(self.population_size, || {
            Route::randomized(self.locations.to_owned(), rng)
        });
        population
    }

    fn next_generation(
        &self,
        population: &mut Vec<Route>,
        mating_pool: &[Route],
        rng: &mut ThreadRng,
    ) {
        population.clear();

        for i in 0..self.population_size / 5 {
            population.push(mating_pool[i % 2].clone());
        }
        self.mutate(population, 0.0, rng);

        self.crossover(population, mating_pool, rng);

        let mutation_threshold_distance = mating_pool[mating_pool.len() - 1].distance;
        self.mutate(population, mutation_threshold_distance, rng);

        // add mating pool back to the population (the only survivors from the previous generation)
        for route in mating_pool {
            population.push(route.clone());
        }
    }

    fn crossover(&self, population: &mut Vec<Route>, mating_pool: &[Route], rng: &mut ThreadRng) {
        let children_count = self.population_size - mating_pool.len();
        let mut shuffling_mating_pool = mating_pool.to_owned();

        'mating: loop {
            let children = shuffling_mating_pool
                .windows(2)
                .map(|couple| Simulation::mate(couple, rng));

            for child in children {
                population.push(child);
                if population.len() >= children_count {
                    break 'mating;
                }
            }

            shuffling_mating_pool.shuffle(rng);
        }
    }

    fn mate(couple: &[Route], rng: &mut ThreadRng) -> Route {
        let parent_x = &couple[0].locations;
        let parent_y = &couple[1].locations;
        let length = parent_x.len();
        let mut offspring = Vec::<Location>::with_capacity(length);

        let slice_size_adjustment = match length {
            0..=4 => 2,
            5..=10 => {
                if rng.gen_bool(0.667) {
                    3
                } else {
                    2
                }
            }
            _ => {
                if rng.gen_bool(0.667) {
                    4
                } else if rng.gen_bool(0.667) {
                    3
                } else {
                    2
                }
            }
        };
        let parent_x_dna_slice_start = rng.gen_range(0, length - slice_size_adjustment);
        let parent_x_dna_slice_end = (parent_x_dna_slice_start
            + rng.gen_range(slice_size_adjustment, (length / 2) + slice_size_adjustment))
        .min(length);
        let parent_x_dna_slice = &parent_x[parent_x_dna_slice_start..parent_x_dna_slice_end];

        let mut recombined = false;
        for y_location in parent_y {
            if !parent_x_dna_slice.contains(y_location) {
                offspring.push(y_location.clone());
            } else if !recombined && (rng.gen_bool(0.10) || y_location == &parent_x_dna_slice[0]) {
                // recombination has a small chance of occurring early instead of trying to attach
                // the the DNA slice with the same gene than the other parent, to prevent a fast
                // convergence to a local maximum and search for other possible solutions
                for x_dna_slice_location in parent_x_dna_slice {
                    offspring.push(x_dna_slice_location.clone())
                }
                recombined = true;
            }
        }
        Route::new(offspring)
    }

    fn mutate(
        &self,
        population: &mut [Route],
        mutation_threshold_distance: f64,
        rng: &mut ThreadRng,
    ) {
        let route_length = self.locations.len();

        let single_mutation_swaps = 1;
        let small_mutation_swaps = ((route_length + 1) / 6).max(1);
        let medium_mutation_swaps = ((route_length + 1) / 4).max(2);
        let big_mutation_swaps = ((route_length + 1) / 2).max(3);

        for route in population {
            if route.distance > mutation_threshold_distance {
                if rng.gen_bool(0.667) {
                    // highest-chance of single mutation
                    Simulation::swap_genes(single_mutation_swaps, route, route_length, rng);
                    route.distance = locations_distance(&route.locations);
                } else if rng.gen_bool(0.667) {
                    // high-chance of small mutation
                    Simulation::swap_genes(small_mutation_swaps, route, route_length, rng);
                    route.distance = locations_distance(&route.locations);
                } else if rng.gen_bool(0.667) {
                    // smaller chance of bigger mutation
                    Simulation::swap_genes(medium_mutation_swaps, route, route_length, rng);
                    route.distance = locations_distance(&route.locations);
                } else {
                    // yet smaller chance of yet bigger mutation
                    Simulation::swap_genes(big_mutation_swaps, route, route_length, rng);
                    route.distance = locations_distance(&route.locations);
                }
            }
        }
    }

    fn swap_genes(n: usize, route: &mut Route, route_length: usize, rng: &mut ThreadRng) {
        for _ in 0..n {
            let i1 = rng.gen_range(0, route_length);
            let i2 = rng.gen_range(0, route_length);
            route.locations.swap(i1, i2);
        }
    }

    fn allocate_mating_pool(population: &[Route]) -> Vec<Route> {
        let mate0 = population[0].clone();
        let mate1 = population[1].clone();
        let mate2 = population[2].clone();
        let mate3 = population[3].clone();
        let mate4 = population[4].clone();
        let mate5 = population[5].clone();
        let mate6 = population[6].clone();

        let mating_pool = vec![mate0, mate1, mate2, mate3, mate4, mate5, mate6];
        debug_assert_eq!(mating_pool.len(), Simulation::MATING_POOL_SIZE);
        mating_pool
    }

    fn select_mating_pool(population: &[Route], mating_pool: &mut [Route]) {
        debug_assert_eq!(mating_pool.len(), Simulation::MATING_POOL_SIZE);

        for route in population {
            if route.distance < mating_pool[0].distance {
                mating_pool[0..7].rotate_right(1);
                mating_pool[0] = route.clone();
            } else if route.distance < mating_pool[1].distance && !mating_pool[0..1].contains(route)
            {
                mating_pool[1..7].rotate_right(1);
                mating_pool[1] = route.clone();
            } else if route.distance < mating_pool[2].distance && !mating_pool[0..2].contains(route)
            {
                mating_pool[2..7].rotate_right(1);
                mating_pool[2] = route.clone();
            } else if route.distance < mating_pool[3].distance && !mating_pool[0..3].contains(route)
            {
                mating_pool[3..7].rotate_right(1);
                mating_pool[3] = route.clone();
            } else if route.distance < mating_pool[4].distance && !mating_pool[0..4].contains(route)
            {
                mating_pool[4..7].rotate_right(1);
                mating_pool[4] = route.clone();
            } else if route.distance < mating_pool[5].distance && !mating_pool[0..5].contains(route)
            {
                mating_pool[5..7].rotate_right(1);
                mating_pool[5] = route.clone();
            } else if route.distance < mating_pool[6].distance && !mating_pool[0..6].contains(route)
            {
                mating_pool[6] = route.clone();
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulate_2_locations() {
        let locations = vec![
            Location {
                name: "A".to_owned(),
                x: 0.0,
                y: 0.0,
            },
            Location {
                name: "B".to_owned(),
                x: 0.0,
                y: 10.0,
            },
        ];

        let simulation = Simulation::new(locations.to_owned());
        let solution = simulation.run(&Arc::new(AtomicBool::default()), |_| {});
        assert_eq!(solution, Route::new(locations))
    }
}
