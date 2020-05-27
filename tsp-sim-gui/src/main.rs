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
    simulation_running: bool,
}

const DEFAULT_LOCATIONS_RON: &'static str = r#"[
  (name: "A", x: 0.0, y: 100.0),
  (name: "B", x: 100.0, y: 0.0),
  (name: "C", x: 100.0, y: 100.0),
]"#;

impl App {
    pub fn new() -> Self {
        App {
            locations_ron: DEFAULT_LOCATIONS_RON.to_owned(),
            locations: ron::de::from_str(DEFAULT_LOCATIONS_RON).unwrap(),
            route: vec!["A".to_owned(), "B".to_owned(), "C".to_owned()],
            simulation_running: false,
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
                println!("Start");
                if !started.compare_and_swap(false, true, Ordering::Relaxed) {
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
            }
            Ok(SimulationCommand::Stop) => stop.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
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

fn gui(
    ui: &mut conrod_core::UiCell,
    ids: &mut Ids,
    app: &mut App,
    simulation_event: &Option<SimulationEvent>,
    command_sender: &Sender<SimulationCommand>,
) {
    use conrod_core::{color, widget, Colorable, Positionable, Sizeable, Widget};

    const MARGIN: conrod_core::Scalar = 7.0;

    match simulation_event {
        Some(SimulationEvent::NewChampion(route)) => {
            app.route = route
                .locations
                .iter()
                .map(|location| location.name.clone())
                .collect()
        }
        Some(SimulationEvent::Started) => app.simulation_running = true,
        Some(SimulationEvent::Finished) => app.simulation_running = false,
        _ => {}
    }

    const TITLE: &'static str = "Hola";
    widget::Canvas::new()
        .pad(MARGIN)
        .color(color::BLACK)
        .set(ids.main_canvas, ui);

    widget::Text::new(TITLE)
        .font_size(16)
        .mid_top_of(ids.main_canvas)
        .set(ids.title, ui);

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

        app.locations_ron = new_locations_ron;
        let _ = ron::de::from_str::<Vec<Location>>(&app.locations_ron)
            .map(|locations| app.locations = locations);

        app.route = app
            .locations
            .iter()
            .map(|location| location.name.clone())
            .collect();
    }

    // Simulation control button
    if app.simulation_running {
        stop_simulation_button(ui, ids, command_sender);
    } else {
        start_simulation_button(ui, ids, app, command_sender);
    }

    // Locations

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
            .send(SimulationCommand::Start(Simulation::new(
                app.locations.clone(),
            )))
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
        title,
        simulation_canvas,

        // controls
        controls_canvas,
        locations_ron_textedit,
        simulate_button,

        // locations
        locations_canvas,
        location_circles[],
        route_lines[],
    }
}
