use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{mpsc, Arc};
use std::thread;
use std::thread::JoinHandle;
use tsp_sim_agent::{GeneticSimulation, Location, Route, Simulation, SimulationEvent};

const NUM_THREADS: usize = 2;

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
        assert!(NUM_THREADS > 0);

        let controls: Vec<(usize, ThreadControl)> = (0..NUM_THREADS)
            .map(|index| (index, self.spawn_simulation_agent()))
            .collect();

        let thread_count = controls.len();
        let mut started_count: usize = 0;
        let mut finished_count: usize = 0;
        let mut iterations: [usize; NUM_THREADS] = [0; NUM_THREADS];
        let mut champion = Route {
            locations: vec![],
            distance: f64::MAX,
        };

        loop {
            let simulation_events: Vec<(usize, SimulationEvent)> = controls
                .iter()
                .filter_map(|(index, control)| {
                    control
                        .event_receiver
                        .try_recv()
                        .ok()
                        .map(|event| (index.clone(), event))
                })
                .collect();

            for (index, simulation_event) in simulation_events {
                match simulation_event {
                    SimulationEvent::Started => {
                        started_count += 1;
                        if started_count >= thread_count {
                            simulation_event_callback(SimulationEvent::Started);
                        }
                    }
                    SimulationEvent::Finished => {
                        finished_count += 1;
                    }
                    SimulationEvent::Iteration(iteration) => {
                        iterations[index] = iteration;
                        let iterations = iterations.iter().sum();
                        simulation_event_callback(SimulationEvent::Iteration(iterations));
                    }
                    SimulationEvent::NewChampion(route, iteration) => {
                        iterations[index] = iteration;
                        if route.distance < champion.distance {
                            let iterations = iterations.iter().sum();
                            champion = route;
                            simulation_event_callback(SimulationEvent::NewChampion(
                                champion.clone(),
                                iterations,
                            ));
                        }
                    }
                }
            }

            if finished_count >= thread_count {
                break;
            }

            if stop.load(Ordering::Relaxed) {
                break;
            }
        }

        simulation_event_callback(SimulationEvent::Finished);

        controls
            .iter()
            .for_each(|(_, control)| control.stop.store(true, Ordering::Relaxed));

        let mut routes: Vec<Route> = controls
            .into_iter()
            .map(|(_, control)| control.join_handle.join().unwrap())
            .collect();

        routes.sort_by(|r1, r2| r1.distance.total_cmp(&r2.distance));
        routes[0].clone()
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
