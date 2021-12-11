#[macro_use]
extern crate conrod_core;
extern crate conrod_gfx;
extern crate conrod_winit;
extern crate gfx;
extern crate gfx_core;
extern crate gfx_window_glutin;
extern crate glutin;
extern crate itertools;
extern crate rand;
extern crate ron;
extern crate serde;
extern crate winit;

extern crate tsp_sim_agent;

use gfx::Device;
use itertools::Itertools;

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc};
use std::thread;
use tsp_sim_agent::{Location, Simulation, SimulationEvent};

const WIN_W: u32 = 800;
const WIN_H: u32 = 600;
const CLEAR_COLOR: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

type DepthFormat = gfx::format::DepthStencil;

// A wrapper around the winit window that allows us to implement the trait necessary for enabling
// the winit <-> conrod conversion functions.
struct WindowRef<'a>(&'a winit::Window);

// Implement the `WinitWindow` trait for `WindowRef` to allow for generating compatible conversion
// functions.
impl<'a> conrod_winit::WinitWindow for WindowRef<'a> {
    fn get_inner_size(&self) -> Option<(u32, u32)> {
        winit::Window::get_inner_size(&self.0).map(Into::into)
    }
    fn hidpi_factor(&self) -> f32 {
        winit::Window::get_hidpi_factor(&self.0) as _
    }
}

// Generate the winit <-> conrod_core type conversion fns.
conrod_winit::conversion_fns!();

// Application state
pub struct App {
    locations_ron: String,
    locations: Vec<Location>,
    route: Vec<String>,
    route_distance: f64,
    route_iteration: usize,
    simulation_running: bool,
    population: usize,
    total_iterations: usize,
}

impl App {
    pub fn new() -> Self {
        let locations: Vec<Location> = ron::de::from_str(EXAMPLE1_RON).unwrap();
        App {
            locations_ron: EXAMPLE1_RON.to_owned(),
            route: locations_names(&locations),
            locations,
            route_distance: f64::NAN,
            route_iteration: 0,
            simulation_running: false,
            population: 200,
            total_iterations: 0,
        }
    }
}

fn main() {
    let builder = glutin::WindowBuilder::new()
        .with_title("TSP simulator")
        .with_dimensions((WIN_W, WIN_H).into());

    let context = glutin::ContextBuilder::new().with_multisampling(4);

    let mut events_loop = winit::EventsLoop::new();

    // Initialize gfx things
    let (context, mut device, mut factory, rtv, _) = gfx_window_glutin::init::<
        conrod_gfx::ColorFormat,
        DepthFormat,
    >(builder, context, &events_loop)
    .unwrap();

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();

    let mut renderer = conrod_gfx::Renderer::new(
        &mut factory,
        &rtv,
        context.window().get_hidpi_factor() as f64,
    )
    .unwrap();

    // Create Ui and Ids of widgets to instantiate
    let mut ui = conrod_core::UiBuilder::new([WIN_W as f64, WIN_H as f64])
        .theme(theme())
        .build();

    let mut ids = Ids::new(ui.widget_id_generator());

    // Load font from file
    let assets = find_folder::Search::KidsThenParents(3, 5)
        .for_folder("assets")
        .unwrap();
    let font_path = assets.join("fonts/NotoSans/NotoSans-Regular.ttf");
    ui.fonts.insert_from_file(font_path).unwrap();

    let image_map = conrod_core::image::Map::new();

    // Application state
    let mut app = App::new();

    // Simulation thread
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    thread::spawn(move || simulation_control_loop(command_receiver, event_sender));

    'main: loop {
        // If the window is closed, this will be None for one tick, so to avoid panicking with
        // unwrap, instead break the loop
        let (win_w, win_h): (u32, u32) = match context.window().get_inner_size() {
            Some(s) => s.into(),
            None => break 'main,
        };

        let dpi_factor = context.window().get_hidpi_factor() as f32;

        if let Some(primitives) = ui.draw_if_changed() {
            let dims = (win_w as f32 * dpi_factor, win_h as f32 * dpi_factor);

            //Clear the window
            renderer.clear(&mut encoder, CLEAR_COLOR);

            renderer.fill(
                &mut encoder,
                dims,
                dpi_factor as f64,
                primitives,
                &image_map,
            );

            renderer.draw(&mut factory, &mut encoder, &image_map);

            encoder.flush(&mut device);
            context.swap_buffers().unwrap();
            device.cleanup();
        }

        let mut should_quit = false;
        events_loop.poll_events(|event| {
            // Convert winit event to conrod event, requires conrod to be built with the `winit` feature
            if let Some(event) = convert_event(event.clone(), &WindowRef(context.window())) {
                ui.handle_event(event);
            }

            // Close window if the escape key or the exit button is pressed
            match event {
                winit::Event::WindowEvent { event, .. } => match event {
                    winit::WindowEvent::KeyboardInput {
                        input:
                            winit::KeyboardInput {
                                virtual_keycode: Some(winit::VirtualKeyCode::Escape),
                                ..
                            },
                        ..
                    }
                    | winit::WindowEvent::CloseRequested => should_quit = true,
                    winit::WindowEvent::Resized(logical_size) => {
                        let hidpi_factor = context.window().get_hidpi_factor();
                        let physical_size = logical_size.to_physical(hidpi_factor);
                        context.resize(physical_size);
                        let (new_color, _) = gfx_window_glutin::new_views::<
                            conrod_gfx::ColorFormat,
                            DepthFormat,
                        >(&context);
                        renderer.on_resize(new_color);
                    }
                    _ => {}
                },
                _ => {}
            }
        });

        if should_quit {
            break 'main;
        }

        // Check for events from the simulation thread
        let simulation_event = event_receiver.try_recv().ok();

        // Update widgets if any event has happened
        if ui.global_input().events().next().is_some() || simulation_event.is_some() {
            let mut ui = ui.set_widgets();
            gui(
                &mut ui,
                &mut ids,
                &mut app,
                &simulation_event,
                &command_sender,
            );
        }
    }
}

#[derive(Debug)]
enum SimulationCommand {
    Start(Simulation),
    Stop,
}

fn simulation_control_loop(rx: Receiver<SimulationCommand>, tx: Sender<SimulationEvent>) {
    let started = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    loop {
        let command = rx.recv();
        match command {
            Ok(SimulationCommand::Start(simulation)) => {
                println!("Start: {:#?}", simulation);
                match started.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(previous_value) => {
                        if !previous_value {
                            start_simulation_thread(&tx, &started, &stop, simulation);
                        }
                    },
                    _ => ()
                }
            }
            Ok(SimulationCommand::Stop) => stop.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
}

fn start_simulation_thread(tx: &Sender<SimulationEvent>, started: &Arc<AtomicBool>, stop: &Arc<AtomicBool>, simulation: Simulation) {
    let tx2 = tx.clone();
    let started2 = started.clone();
    let stop2 = stop.clone();
    thread::spawn(move || {
        println!("...started simulation thread");
        simulation.run(&stop2, |event| tx2.send(event).unwrap());
        println!("...simulation thread is done");
        started2.store(false, Ordering::Relaxed);
        stop2.store(false, Ordering::Relaxed);
    });
}

fn theme() -> conrod_core::Theme {
    use conrod_core::position::{Align, Direction, Padding, Position, Relative};
    conrod_core::Theme {
        name: "Demo Theme".to_string(),
        padding: Padding::none(),
        x_position: Position::Relative(Relative::Align(Align::Start), None),
        y_position: Position::Relative(Relative::Direction(Direction::Backwards, 20.0), None),
        background_color: conrod_core::color::DARK_CHARCOAL,
        shape_color: conrod_core::color::LIGHT_CHARCOAL,
        border_color: conrod_core::color::BLACK,
        border_width: 0.0,
        label_color: conrod_core::color::WHITE,
        font_id: None,
        font_size_large: 26,
        font_size_medium: 18,
        font_size_small: 12,
        widget_styling: conrod_core::theme::StyleMap::default(),
        mouse_drag_threshold: 0.0,
        double_click_threshold: std::time::Duration::from_millis(500),
    }
}

const EXAMPLE1_RON: &'static str = r#"[
  (name: "A", x: 0.0, y: 100.0),
  (name: "B", x: 100.0, y: 0.0),
  (name: "C", x: 100.0, y: 100.0),
  (name: "D", x: 200.0, y: 200.0),
  (name: "E", x: -200.0, y: -100.0),
  (name: "F", x: 0.0, y: 200.0),
  (name: "G", x: 150.0, y: 250.0),
  (name: "H", x: 10.0, y: -200.0),
  (name: "I", x: -150.0, y: 250.0),
  (name: "J", x: 200.0, y: -200.0),
  (name: "K", x: -200.0, y: 100.0),
  (name: "L", x: -100.0, y: 100.0),
  (name: "M", x: 170.0, y: -50.0),
  (name: "N", x: -120.0, y: -210.0),
  (name: "O", x: 50.0, y: -50.0),
  (name: "P", x: -50.00, y: -50.0),
]"#;

const EXAMPLE2_RON: &'static str = r#"[
  (name: "A", x: 200.0, y: 200.0),
  (name: "B", x: -200.0, y: -200.0),
  (name: "C", x: -200.0, y:  200.0),
  (name: "D", x: 200.0, y: -200.0),
  (name: "E", x: 200.0, y: 0.0),
  (name: "F", x: 0.0, y: 200.0),
  (name: "G", x: 0.0, y: -200.0),
  (name: "H", x: -200.0, y: 0.0),
  (name: "I", x: 0.0, y: 0.0),
]"#;

const EXAMPLE3_RON: &'static str = r#"[
  (name: "A", x: 200.0, y: 200.0),
  (name: "B", x: -200.0, y: -200.0),
  (name: "C", x: -200.0, y:  200.0),
  (name: "D", x: 200.0, y: -200.0),
  (name: "E", x: 200.0, y: 0.0),
  (name: "F", x: 0.0, y: 200.0),
  (name: "G", x: 0.0, y: -200.0),
  (name: "H", x: -200.0, y: 0.0),
  (name: "I", x: -200.0, y: 100.0),
  (name: "J", x: -100.0, y: 200.0),
  (name: "K", x: 100.0, y: 200.0),
  (name: "L", x: 200.0, y: 100.0),
  (name: "M", x: 200.0, y: -100.0),
  (name: "N", x: 100.0, y: -200.0),
  (name: "O", x: -100.0, y: -200.0),
  (name: "P", x: -200.0, y: -100.0),
]"#;

fn locations_names(locations: &[Location]) -> Vec<String> {
    locations
        .iter()
        .map(|location| location.name.clone())
        .collect()
}

fn gui(
    ui: &mut conrod_core::UiCell,
    ids: &mut Ids,
    app: &mut App,
    simulation_event: &Option<SimulationEvent>,
    command_sender: &Sender<SimulationCommand>,
) {
    use conrod_core::{
        color, position, widget, Colorable, Labelable, Positionable, Sizeable, Widget,
    };

    const MARGIN: conrod_core::Scalar = 7.0;

    match simulation_event {
        Some(SimulationEvent::Iteration(iteration)) => {
            app.total_iterations = iteration.clone();
        }
        Some(SimulationEvent::NewChampion(route, iteration)) => {
            app.route = locations_names(&route.locations);
            app.route_distance = route.distance;
            app.route_iteration = iteration.clone();
        }
        Some(SimulationEvent::Started) => app.simulation_running = true,
        Some(SimulationEvent::Finished) => app.simulation_running = false,
        _ => {}
    }

    widget::Canvas::new()
        .pad(MARGIN)
        .color(color::BLACK)
        .set(ids.main_canvas, ui);

    widget::Text::new(&format!("Distance: {:.3}", app.route_distance))
        .font_size(16)
        .top_left_with_margins_on(ids.main_canvas, 0.0, MARGIN)
        .set(ids.distance_label, ui);

    widget::Text::new(&format!("Iterations: {:06}", app.total_iterations))
        .font_size(16)
        .mid_top_of(ids.main_canvas)
        .set(ids.total_iterations_label, ui);

    // Example buttons

    for _press in widget::Button::new()
        .label("3")
        .label_font_size(14)
        .label_y(position::Relative::Scalar(2.0))
        .color(color::RED)
        .hover_color(color::DARK_RED)
        .press_color(color::LIGHT_RED)
        .label_color(color::DARK_YELLOW)
        .top_right_with_margins_on(ids.main_canvas, 0.0, MARGIN)
        .w_h(30.0, 20.0)
        .set(ids.example_3_button, ui)
    {
        if app.simulation_running {
            break;
        };

        set_locations_input(app, EXAMPLE3_RON.to_string());
    }

    for _press in widget::Button::new()
        .label("2")
        .label_font_size(14)
        .label_y(position::Relative::Scalar(2.0))
        .color(color::RED)
        .hover_color(color::DARK_RED)
        .press_color(color::LIGHT_RED)
        .label_color(color::DARK_YELLOW)
        .left_from(ids.example_3_button, 5.0)
        .w_h(30.0, 20.0)
        .set(ids.example_2_button, ui)
    {
        if app.simulation_running {
            break;
        };

        set_locations_input(app, EXAMPLE2_RON.to_string());
    }

    for _press in widget::Button::new()
        .label("1")
        .label_font_size(14)
        .label_y(position::Relative::Scalar(2.0))
        .color(color::RED)
        .hover_color(color::DARK_RED)
        .press_color(color::LIGHT_RED)
        .label_color(color::DARK_YELLOW)
        .left_from(ids.example_2_button, 5.0)
        .w_h(30.0, 20.0)
        .set(ids.example_1_button, ui)
    {
        if app.simulation_running {
            break;
        };

        set_locations_input(app, EXAMPLE1_RON.to_string());
    }

    widget::Text::new("Examples:")
        .font_size(16)
        .left_from(ids.example_2_button, 40.0)
        .set(ids.examples_label, ui);

    // Controls and locations areas

    widget::Canvas::new()
        .down(0.0)
        .align_middle_x_of(ids.main_canvas)
        .kid_area_w_of(ids.main_canvas)
        .kid_area_h_of(ids.main_canvas)
        .color(color::TRANSPARENT)
        .pad(MARGIN)
        .flow_right(&[
            (
                ids.controls_canvas,
                widget::Canvas::new()
                    .color(color::DARK_CHARCOAL)
                    .length(250.0),
            ),
            (
                ids.locations_canvas,
                widget::Canvas::new().color(color::DARK_GREY),
            ),
        ])
        .set(ids.simulation_canvas, ui);

    for new_locations_ron in widget::TextEdit::new(&app.locations_ron)
        .mid_top_of(ids.controls_canvas)
        .w_of(ids.controls_canvas)
        .h(400.0)
        .color(color::YELLOW)
        .font_size(14)
        .set(ids.locations_ron_textedit, ui)
    {
        if app.simulation_running {
            break;
        };

        set_locations_input(app, new_locations_ron);
    }

    // Population input
    widget::Text::new(" Population")
        .font_size(16)
        .down_from(ids.locations_ron_textedit, 10.0)
        .set(ids.population_label, ui);

    for new_population_event in widget::TextBox::new(&format!("{}", app.population))
        .font_size(16)
        .w(150.0)
        .h(20.0)
        .right_from(ids.population_label, 4.0)
        .set(ids.population_textbox, ui)
    {
        if app.simulation_running {
            break;
        };

        match new_population_event {
            widget::text_box::Event::Update(new_population) => {
                let _ =
                    usize::from_str(&new_population).map(|population| app.population = population);
            }
            _ => {}
        }
    }

    // Simulation control button
    if app.simulation_running {
        stop_simulation_button(ui, ids, command_sender);
    } else {
        start_simulation_button(ui, ids, app, command_sender);
    }

    // Locations
    widget::Text::new(&format!("Iteration: {:06}", app.route_iteration))
        .font_size(12)
        .color(color::DARK_BROWN)
        .top_right_with_margins_on(ids.locations_canvas, 0.0, MARGIN)
        .set(ids.route_iteration_label, ui);

    ids.location_circles
        .resize(app.locations.len(), &mut ui.widget_id_generator());

    for (&id, location) in ids.location_circles.iter().zip(&app.locations) {
        widget::Circle::fill(5.0)
            .x_relative_to(ids.locations_canvas, location.x)
            .y_relative_to(ids.locations_canvas, location.y)
            .color(color::RED)
            .set(id, ui);
    }

    // Route

    let lines: Vec<(&Location, &Location)> = app
        .route
        .iter()
        .tuple_windows()
        .filter_map(|(from, to)| {
            let mut from_location: Option<&Location> = None;
            let mut to_location: Option<&Location> = None;
            for location in &app.locations {
                if location.name.eq(from) {
                    from_location = Some(location);
                } else if location.name.eq(to) {
                    to_location = Some(location);
                }

                if from_location.is_some() && to_location.is_some() {
                    break;
                }
            }
            from_location.and_then(|from| to_location.map(|to| (from, to)))
        })
        .collect();

    ids.route_lines
        .resize(lines.len(), &mut ui.widget_id_generator());

    for (&id, (from, to)) in ids.route_lines.iter().zip(lines) {
        let start = [from.x, from.y];
        let end = [to.x, to.y];
        widget::Line::centred(start, end)
            .x_relative_to(ids.locations_canvas, (from.x + to.x) / 2.0)
            .y_relative_to(ids.locations_canvas, (from.y + to.y) / 2.0)
            .color(color::RED)
            .set(id, ui);
    }
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

fn start_simulation_button(
    ui: &mut conrod_core::UiCell,
    ids: &mut Ids,
    app: &mut App,
    command_sender: &Sender<SimulationCommand>,
) {
    use conrod_core::{widget, Labelable, Positionable, Sizeable, Widget};
    for _press in widget::Button::new()
        .label("START")
        .mid_bottom_with_margin_on(ids.controls_canvas, 10.0)
        .w_h(130.0, 65.0)
        .set(ids.simulate_button, ui)
    {
        command_sender
            .send(SimulationCommand::Start(Simulation {
                population_size: app.population,
                ..Simulation::new(app.locations.clone())
            }))
            .unwrap();
    }
}

fn stop_simulation_button(
    ui: &mut conrod_core::UiCell,
    ids: &mut Ids,
    command_sender: &Sender<SimulationCommand>,
) {
    use conrod_core::{color, widget, Colorable, Labelable, Positionable, Sizeable, Widget};
    for _press in widget::Button::new()
        .label("STOP")
        .color(color::RED)
        .hover_color(color::DARK_RED)
        .press_color(color::LIGHT_RED)
        .label_color(color::DARK_YELLOW)
        .mid_bottom_with_margin_on(ids.controls_canvas, 10.0)
        .w_h(130.0, 65.0)
        .set(ids.simulate_button, ui)
    {
        command_sender.send(SimulationCommand::Stop).unwrap();
    }
}

widget_ids! {
    pub struct Ids {
        main_canvas,
        distance_label,
        total_iterations_label,
        examples_label,
        example_1_button,
        example_2_button,
        example_3_button,
        simulation_canvas,

        // controls
        controls_canvas,
        locations_ron_textedit,
        population_label,
        population_textbox,
        simulate_button,

        // locations
        locations_canvas,
        route_iteration_label,
        location_circles[],
        route_lines[],
    }
}
