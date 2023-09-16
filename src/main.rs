use std::{thread, time::Duration};

use anyhow::{anyhow, Result};

use cpal::traits::{DeviceTrait, HostTrait};
use handlers::background_layer::BackgroundLayer;
use sctk::{
    output::OutputHandler,
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop, channel,
    },
};
use spectrum_analyzer::{
    samples_fft_to_spectrum, scaling::{divide_by_N_sqrt, self, SpectrumDataStats}, windows::hann_window, FrequencyLimit,
};
use wayland_client::{globals::registry_queue_init, Connection, WaylandSource};

mod handlers;
mod renderer;

const FPS: f32 = 60.;
const MSPF: f32 = 1000. / FPS;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // first get connection to wayland
    let conn = Connection::connect_to_env().unwrap();

    // now set up main handler
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    // init state, do roundtrip to get display info
    let mut bg = BackgroundLayer::new(&globals, &qh)?;

    event_queue.roundtrip(&mut bg).unwrap();

    let requested_display = std::env::args().nth(1).expect("no display given");

    match bg.output_state().outputs().find_map(|output| {
        let output_info = bg.output_state().info(&output).unwrap();
        if output_info.clone().name.unwrap() != requested_display {
            None
        } else {
            Some(output)
        }
    }) {
        Some(output) => {
            bg.create_layer(&qh, output);
        }
        None => return Err(anyhow!("couldn't find display")),
    };

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
                let hann_window = hann_window(&d[0..1024]);
                // calc spectrum
                let spectrum_hann_window = samples_fft_to_spectrum(
                    // (windowed) samples
                    &hann_window,
                    // sampling rate
                    44100,
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

    loop {
        // dispatch. 5000ms is random, does it matter?
        event_loop.run(Duration::from_millis(50), &mut bg, |bg| {
            if let Ok(mut d) = rx.try_recv() {
                //let mut buf = vec![Default::default(); d.data().len() as usize];
                //d.apply_scaling_fn(&scaling::scale_to_zero_to_one, &mut buf).unwrap();
                //dbg!(d.range());
                //if d.range() < 0.1.into() {
                //    return
                //}

                let (max_f, max_fv) = d.max();
                let hmm = max_f / d.max_fr();
                let med_fv = d.median();
                bg.set_fft(max_fv.val(), max_fv.val());
            }
        })?;

        if bg.exit {
            println!("how tho");
            println!("exiting example");
            break;
        }
    }

    Ok(())
}
