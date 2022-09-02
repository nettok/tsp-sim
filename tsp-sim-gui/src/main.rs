#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

extern crate itertools;
extern crate ron;

mod examples;

use anyhow::Result;
use eframe::{egui, emath::pos2, epaint::Color32, epaint::Stroke};
use itertools::Itertools;

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc};
use std::thread;

use tsp_sim_agent::{Location, Simulation, SimulationEvent};

fn main() -> Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "TSP simulator",
        options,
        Box::new(|cc| {
            let egui_ctx = cc.egui_ctx.clone();

            let (command_sender, command_receiver) = mpsc::channel();
            let (event_sender, event_receiver) = mpsc::channel();
            thread::spawn(move || {
                simulation_control_loop(command_receiver, event_sender, egui_ctx)
            });

            Box::new(App::new(command_sender, event_receiver))
        }),
    );
    Ok(())
}

// Application state
pub struct App {
    locations_ron: String,
    locations: Vec<Location>,
    route: Vec<String>,
    route_distance: f64,
    route_iteration: usize,
    simulation_running: bool,
    population_text: String,
    population: usize,
    total_iterations: usize,

    // Simulation thread events and control
    command_sender: Sender<SimulationCommand>,
    event_receiver: Receiver<SimulationEvent>,
}

impl App {
    fn new(
        command_sender: Sender<SimulationCommand>,
        event_receiver: Receiver<SimulationEvent>,
    ) -> Self {
        let locations: Vec<Location> = ron::de::from_str(examples::EXAMPLE1_RON).unwrap();
        Self {
            locations_ron: examples::EXAMPLE1_RON.to_string(),
            route: locations_names(&locations),
            locations,
            route_distance: f64::NAN,
            route_iteration: 0,
            simulation_running: false,
            population_text: "200".to_string(),
            population: 200,
            total_iterations: 0,

            command_sender,
            event_receiver,
        }
    }
}

fn locations_names(locations: &[Location]) -> Vec<String> {
    locations
        .iter()
        .map(|location| location.name.clone())
        .collect()
}

fn set_locations_input(app: &mut App, new_locations_ron: String) {
    app.locations_ron = new_locations_ron;
    let _ = ron::de::from_str::<Vec<Location>>(&app.locations_ron)
        .map(|locations| app.locations = locations);

    app.route = locations_names(&app.locations);
    app.route_distance = f64::NAN;
    app.route_iteration = 0;
    app.total_iterations = 0;
}

// Simulation

#[derive(Debug)]
enum SimulationCommand {
    Start(Simulation),
    Stop,
}

fn simulation_control_loop(
    rx: Receiver<SimulationCommand>,
    tx: Sender<SimulationEvent>,
    egui_ctx: egui::Context,
) {
    let started = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    loop {
        let command = rx.recv();
        match command {
            Ok(SimulationCommand::Start(simulation)) => {
                println!("Start: {:#?}", simulation);
                if let Ok(previous_value) =
                    started.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                {
                    if !previous_value {
                        start_simulation_thread(&tx, &started, &stop, simulation, egui_ctx.clone());
                    }
                }
            }
            Ok(SimulationCommand::Stop) => stop.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
}

fn start_simulation_thread(
    tx: &Sender<SimulationEvent>,
    started: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    simulation: Simulation,
    egui_ctx: egui::Context,
) {
    let tx2 = tx.clone();
    let started2 = started.clone();
    let stop2 = stop.clone();
    thread::spawn(move || {
        println!("...started simulation thread");
        simulation.run(&stop2, |event| {
            tx2.send(event).unwrap();
            egui_ctx.request_repaint();
        });
        println!("...simulation thread is done");
        started2.store(false, Ordering::Relaxed);
        stop2.store(false, Ordering::Relaxed);
    });
}

// GUI

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for events from the simulation thread
        let simulation_event = self.event_receiver.try_recv().ok();

        match simulation_event {
            Some(SimulationEvent::Iteration(iteration)) => {
                self.total_iterations = iteration;
            }
            Some(SimulationEvent::NewChampion(route, iteration)) => {
                self.route = locations_names(&route.locations);
                self.locations = route.locations;
                self.route_distance = route.distance;
                self.route_iteration = iteration;
            }
            Some(SimulationEvent::Started) => self.simulation_running = true,
            Some(SimulationEvent::Finished) => self.simulation_running = false,
            _ => {}
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Distance: {:.3}", self.route_distance));
                ui.separator();
                ui.label(format!("Iterations: {:06}", self.total_iterations));
                ui.separator();
                ui.add_enabled_ui(!self.simulation_running, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Examples:");
                        if ui.small_button("1").clicked() {
                            set_locations_input(self, examples::EXAMPLE1_RON.to_string());
                        }
                        if ui.small_button("2").clicked() {
                            set_locations_input(self, examples::EXAMPLE2_RON.to_string());
                        }
                        if ui.small_button("3").clicked() {
                            set_locations_input(self, examples::EXAMPLE3_RON.to_string());
                        }
                    });
                });
            });
        });

        egui::SidePanel::left("left_panel").show(ctx, |ui| {
            ui.add_enabled_ui(!self.simulation_running, |ui| {
                if ui.text_edit_multiline(&mut self.locations_ron).changed() {
                    set_locations_input(self, self.locations_ron.to_owned());
                }
                ui.separator();

                ui.label("Population");
                if ui.text_edit_singleline(&mut self.population_text).changed() {
                    if !self.population_text.is_empty() {
                        match usize::from_str(&self.population_text)
                            .map(|population| self.population = population)
                        {
                            Ok(_) => (),
                            Err(_) => self.population_text = self.population.to_string(),
                        }
                    } else {
                        self.population = 0;
                    }
                }
                ui.separator();
            });

            let simulation_control_button_text = if !self.simulation_running {
                "START"
            } else {
                "STOP"
            };
            if ui.button(simulation_control_button_text).clicked() {
                if !self.simulation_running {
                    self.command_sender
                        .send(SimulationCommand::Start(Simulation {
                            population_size: self.population,
                            ..Simulation::new(self.locations.clone())
                        }))
                        .unwrap();
                } else {
                    self.command_sender.send(SimulationCommand::Stop).unwrap();
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(
                egui::Layout::right_to_left(eframe::emath::Align::Min),
                |ui| {
                    ui.label(format!("Iteration: {:06}", self.route_iteration));
                },
            );

            let max_rect = ui.max_rect();
            let max_left_top = max_rect.left_top();
            let x_zero = max_left_top.x + 7.;
            let y_zero = max_left_top.y + 7.;
            let point_radius: f32 = 5.;
            let point_color = Color32::LIGHT_RED;
            let line_color = Color32::LIGHT_RED;

            let painter = ui.painter().with_clip_rect(ui.max_rect());

            let draw_point = |x: f32, y: f32| {
                painter.circle_filled(pos2(x_zero + x, y_zero + y), point_radius, point_color);
            };

            let draw_line = |x1: f32, y1: f32, x2: f32, y2: f32| {
                let from = pos2(x_zero + x1, y_zero + y1);
                let to = pos2(x_zero + x2, y_zero + y2);
                painter.line_segment([from, to], Stroke::new(1., line_color))
            };

            for location in &self.locations {
                draw_point(location.x as f32, location.y as f32);
            }

            for (from, to) in (0..self.locations.len()).tuple_windows() {
                let from_x = self.locations[from].x as f32;
                let from_y = self.locations[from].y as f32;
                let to_x = self.locations[to].x as f32;
                let to_y = self.locations[to].y as f32;
                draw_line(from_x, from_y, to_x, to_y);
            }
        });
    }
}
