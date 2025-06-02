use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{mpsc, Arc};
use std::thread;
use std::thread::JoinHandle;
use tsp_sim_agent::{GeneticSimulation, Location, Route, Simulation, SimulationEvent};

struct ThreadControl {
    event_receiver: Receiver<SimulationEvent>,
    stop: Arc<AtomicBool>,
    join_handle: JoinHandle<Route>,
}

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
        let thread_control = self.spawn_simulation_agent();

        loop {
            let simulation_event = thread_control.event_receiver.try_recv().ok();

            match simulation_event {
                Some(SimulationEvent::Finished) => {
                    break;
                }
                Some(event) => simulation_event_callback(event),
                _ => {}
            }

            if stop.load(Ordering::Relaxed) {
                break;
            }
        }

        simulation_event_callback(SimulationEvent::Finished);
        thread_control.stop.store(true, Ordering::Relaxed);
        thread_control.join_handle.join().unwrap()
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

    fn spawn_simulation_agent(&self) -> ThreadControl {
        let sim = GeneticSimulation::from(self.clone());
        let (event_sender, event_receiver) = mpsc::channel::<SimulationEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let join_handle = thread::spawn(move || {
            sim.run(&stop2, |event| {
                event_sender.send(event).unwrap();
            })
        });

        ThreadControl {
            event_receiver,
            stop,
            join_handle,
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
