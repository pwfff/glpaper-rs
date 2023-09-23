use std::time::Duration;

use anyhow::{anyhow, Result};

use cpal::traits::{DeviceTrait, HostTrait};
use handlers::background_layer::BackgroundLayer;
use sctk::{
    output::OutputHandler,
    reexports::calloop::{
        channel,
        signals::{Signal, Signals},
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
};
use spectrum_analyzer::{
    samples_fft_to_spectrum, scaling::divide_by_N_sqrt, windows::hann_window, FrequencyLimit,
};
use wayland_client::{globals::registry_queue_init, Connection, WaylandSource};

use smithay::{
    backend::{
        renderer::{
            element::{texture::TextureRenderElement, Element, RenderElement},
            gles::GlesTexture,
            glow::GlowRenderer,
            Frame, Renderer,
        },
        winit,
    },
    input::{
        keyboard::{FilterResult, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
        SeatHandler, SeatState,
    },
    utils::{Rectangle, Transform, SERIAL_COUNTER},
};
use smithay_egui::EguiState;

mod handlers;
mod renderer;

const FPS: f32 = 60.;
const MSPF: f32 = 1000. / FPS;

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();

    let shader_id = std::env::args().nth(1);

    // have to init this before tokio or tokio will i guess just eat all our signals forever
    let signal_source = Signals::new(&[Signal::SIGUSR2])?;

    // create a winit-backend
    let (mut backend, mut input) =
        winit::init::<GlowRenderer>().map_err(|_| anyhow::anyhow!("Winit failed to start"))?;
    // create an `EguiState`. Usually this would be part of your global smithay state
    let egui = EguiState::new(Rectangle::from_loc_and_size(
        (0, 0),
        backend.window_size().physical_size.to_logical(1),
    ));

    // you might also need additional structs to store your ui-state, like the demo_lib does
    let mut demo_ui = egui_demo_lib::DemoWindows::default();

    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_seat("seat-0");
    let keyboard = seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    let pointer = seat.add_pointer();

    //tokio::runtime::Builder::new_multi_thread()
    //    .enable_all()
    //    .build()
    //    .unwrap()
    //    .block_on(async {
    // first get connection to wayland
    let conn = Connection::connect_to_env().unwrap();

    // now set up main handler
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    // init state, do roundtrip to get display info
    let mut bg = BackgroundLayer::new(&globals, shader_id, &qh, egui.clone(), seat_state)?;
    keyboard.set_focus(&mut bg, Some(egui.clone()), SERIAL_COUNTER.next_serial());

    event_queue.roundtrip(&mut bg).unwrap();

    for output in bg.output_state().outputs() {
        let output_info = bg.output_state().info(&output).unwrap();
        bg.configure_output(&qh, output, output_info);
    }

    // round trip to get layer we just added configured, rendering will start
    event_queue.roundtrip(&mut bg).unwrap();

    // get a loop, add a timer source so we can draw at limited fps
    let mut event_loop: EventLoop<BackgroundLayer> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");
    let loop_handle = event_loop.handle();

    let mspf_d = Duration::from_millis(MSPF as u64);
    let t = Timer::from_duration(mspf_d);
    loop_handle
        .insert_source(t, move |_, _, bg| {
            bg.draw();
            TimeoutAction::ToDuration(mspf_d)
        })
        .unwrap();

    loop_handle.insert_source(signal_source, |event, _, bg| {
        bg.reset().unwrap();
    })?;

    // add wayland events into the loop
    let ws = WaylandSource::new(event_queue).unwrap();
    ws.insert(loop_handle).unwrap();

    let host = cpal::default_host();
    let dev = host.default_output_device().unwrap();
    let conf = dev.default_output_config().unwrap().config();
    let (tx, rx) = channel::channel();
    let stm = dev
        .build_input_stream(
            &conf,
            move |d: &[f32], f| {
                let hann_window = hann_window(&d[0..(d.len() >> 1).next_power_of_two()]);
                // calc spectrum
                let spectrum_hann_window = samples_fft_to_spectrum(
                    // (windowed) samples
                    &hann_window,
                    // sampling rate
                    conf.sample_rate.0,
                    // optional frequency limit: e.g. only interested in frequencies 50 <= f <= 150?
                    FrequencyLimit::All,
                    // optional scale
                    Some(&divide_by_N_sqrt),
                )
                .unwrap();

                tx.send(spectrum_hann_window).unwrap();

                //for (i, (f, fv)) in spectrum_hann_window.data().iter().enumerate() {
                //    dbg!((f, fv));
                //    if i > 5 {
                //        break;
                //    }
                //}
            },
            |e| {},
            None,
        )
        .unwrap();

    let sig = event_loop.get_signal();

    // dispatch. 5000ms is random, does it matter?
    event_loop.run(Duration::from_millis(1), &mut bg, |bg| {
        if bg.exit {
            sig.stop();
        }

        if let Ok(d) = rx.try_recv() {
            //let mut buf = vec![Default::default(); d.data().len() as usize];
            //d.apply_scaling_fn(&scaling::scale_to_zero_to_one, &mut buf).unwrap();
            //dbg!(d.range());
            //if d.range() < 0.1.into() {
            //    return
            //}
            let mut mel = d.to_mel_map();
            let highs = mel.split_off(&75).split_off(&750);
            let max_l = mel.values().fold(0., |a: f32, x| a.max(*x));
            let max_h = highs.values().fold(0., |a: f32, x| a.max(*x));

            let (max_f, max_fv) = d.max();
            let hmm = max_f / d.max_fr();
            let med_fv = d.median();
            let avg_fv = d.average();
            bg.set_fft(max_l, max_h);
        }

        match input.dispatch_new_events(|event| {
            bg.handle_winit(event, &keyboard, &pointer).unwrap();
        }) {
            Ok(()) => {
                let size = backend.window_size().physical_size;
                // Here we compute the rendered egui frame
                let egui_frame: TextureRenderElement<GlesTexture> = egui
                    .render(
                        |ctx| demo_ui.ui(ctx),
                        backend.renderer(),
                        // Just render it over the whole window, but you may limit the area
                        Rectangle::from_loc_and_size((0, 0), size.to_logical(1)),
                        // we also completely ignore the scale *everywhere* in this example, but egui is HiDPI-ready
                        1.0,
                        1.0,
                    )
                    .expect("Failed to render egui");

                // Lastly put the rendered frame on the screen
                backend.bind().unwrap();
                let renderer = backend.renderer();
                {
                    let mut frame = renderer.render(size, Transform::Flipped180).unwrap();
                    frame
                        .clear(
                            [1.0, 1.0, 1.0, 1.0],
                            &[Rectangle::from_loc_and_size((0, 0), size)],
                        )
                        .unwrap();
                    RenderElement::<GlowRenderer>::draw(
                        &egui_frame,
                        &mut frame,
                        egui_frame.src(),
                        egui_frame.geometry(1.0.into()),
                        &[Rectangle::from_loc_and_size((0, 0), size)],
                    )
                    .unwrap();
                }
                backend.submit(None).unwrap();
            }
            Err(winit::WinitError::WindowClosed) => {
                backend.window().set_visible(false);
            },
        };
    })?;

    //        Ok::<(), anyhow::Error>(())
    //    })?;

    Ok(())
}
