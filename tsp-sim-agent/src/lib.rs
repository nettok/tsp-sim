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
    locations: Vec<Location>,
    population_size: usize,
    max_iterations: Option<usize>,
    assume_convergence: Option<usize>,
}

#[derive(Debug)]
pub enum SimulationEvent {
    Started,
    NewChampion(Route),
    Finished,
}

impl Simulation {
    const MATING_POOL_SIZE: usize = 5;

    pub fn new(locations: Vec<Location>) -> Simulation {
        Simulation {
            locations,
            population_size: 100,
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
            simulation_event_callback(SimulationEvent::NewChampion(champion.to_owned()));
            return champion;
        }

        let mut rng = thread_rng();

        let mut population = self.initial_random_population(&mut rng);
        let mut mating_pool = Simulation::allocate_mating_pool(&population);
        Simulation::select_mating_pool(&population, &mut mating_pool, &mut rng);

        let mut champion = mating_pool[0].to_owned();
        let mut champion_iterations: usize = 0;
        simulation_event_callback(SimulationEvent::NewChampion(champion.to_owned()));

        let max_iterations = self.max_iterations.unwrap_or(usize::MAX);
        let assume_convergence = self.assume_convergence.unwrap_or(usize::MAX);
        let mut iteration: usize = 0;
        loop {
            iteration += 1;
            champion_iterations += 1;
            self.next_generation(&mut population, &mating_pool, &mut rng);
            Simulation::select_mating_pool(&population, &mut mating_pool, &mut rng);
            if champion.distance > mating_pool[0].distance {
                champion = mating_pool[0].to_owned();
                champion_iterations = 0;
                simulation_event_callback(SimulationEvent::NewChampion(champion.to_owned()));
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
        // replace the current population with the children of the mating pool
        population.clear();
        self.crossover(population, mating_pool, rng);
        self.mutate(population, mating_pool, rng);

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
            0..=4 => 1,
            5..=10 => 2,
            _ => 3,
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
            } else if !recombined && y_location == &parent_x_dna_slice[0] {
                for x_dna_slice_location in parent_x_dna_slice {
                    offspring.push(x_dna_slice_location.clone())
                }
                recombined = true;
            }
        }
        Route::new(offspring)
    }

    fn mutate(&self, population: &mut Vec<Route>, mating_pool: &[Route], rng: &mut ThreadRng) {
        // We will mutate any route that would have little chance of being selected as part of the
        // next mating pool to increase the chance of getting an unexpected mutant champion.
        //
        // In this case mating_pool[mating_pool.len() - 2] because the last element is not a
        // champion, but a randomly selected route.
        let mutation_threshold_distance = &mating_pool[mating_pool.len() - 2].distance;
        let route_length = self.locations.len();

        for route in population {
            if route.distance > *mutation_threshold_distance {
                let i1 = rng.gen_range(0, route_length);
                let i2 = rng.gen_range(0, route_length);
                route.locations.swap(i1, i2);
                route.distance = locations_distance(&route.locations);
            }
        }
    }

    fn allocate_mating_pool(population: &[Route]) -> Vec<Route> {
        let mate0 = population[0].clone();
        let mate1 = population[1].clone();
        let mate2 = population[2].clone();
        let mate3 = population[3].clone();
        let mate4 = population[4].clone();

        let mating_pool = vec![mate0, mate1, mate2, mate3, mate4];
        debug_assert_eq!(mating_pool.len(), Simulation::MATING_POOL_SIZE);
        mating_pool
    }

    fn select_mating_pool(population: &[Route], mating_pool: &mut [Route], rng: &mut ThreadRng) {
        debug_assert_eq!(mating_pool.len(), Simulation::MATING_POOL_SIZE);

        for route in population {
            if route.distance < mating_pool[0].distance {
                mating_pool.swap(3, 2);
                mating_pool.swap(2, 1);
                mating_pool.swap(1, 0);
                mating_pool[0] = route.clone();
            } else if route.distance < mating_pool[1].distance {
                mating_pool.swap(3, 2);
                mating_pool.swap(2, 1);
                mating_pool[1] = route.clone();
            } else if route.distance < mating_pool[2].distance {
                mating_pool.swap(3, 2);
                mating_pool[2] = route.clone();
            } else if route.distance < mating_pool[3].distance {
                mating_pool[3] = route.clone();
            }
        }

        // ... and randomly select a route into the last element of the mating pool to reduce the
        // probability of converging into a local maximum instead of a global maximum
        for _ in 0..10 {
            let i = rng.gen_range(0, population.len());
            if !mating_pool.contains(&population[i]) {
                mating_pool[4] = population[i].clone();
                break;
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
