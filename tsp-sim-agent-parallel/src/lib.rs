use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tsp_sim_agent::{GeneticSimulation, Location, Route, Simulation, SimulationEvent};

#[derive(Debug, Clone)]
pub struct ParallelSimulation {
    pub locations: Vec<Location>,
    pub population_size: usize,
    pub max_iterations: Option<usize>,
    pub assume_convergence: Option<usize>,
}

impl Simulation for ParallelSimulation {
    fn run<F>(&self, stop: &Arc<AtomicBool>, simulation_event_callback: F) -> Route
    where
        F: Fn(SimulationEvent),
    {
        let sim = GeneticSimulation::from(self.clone());
        sim.run(stop, simulation_event_callback)
    }
}

impl ParallelSimulation {
    pub fn new(locations: Vec<Location>) -> ParallelSimulation {
        ParallelSimulation {
            locations,
            population_size: 200,
            max_iterations: Some(100_000),
            assume_convergence: Some(25_000),
        }
    }
}

impl From<ParallelSimulation> for GeneticSimulation {
    fn from(parallel: ParallelSimulation) -> Self {
        Self {
            locations: parallel.locations,
            population_size: parallel.population_size,
            max_iterations: parallel.max_iterations,
            assume_convergence: parallel.assume_convergence,
        }
    }
}
