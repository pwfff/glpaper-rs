use std::{thread, time::Duration};

use anyhow::{anyhow, Result};

use handlers::background_layer::BackgroundLayer;
use sctk::{
    output::OutputHandler,
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
};
use wayland_client::{globals::registry_queue_init, Connection, WaylandSource};

mod handlers;
mod renderer;

const FPS: f32 = 10.;
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

    loop {
        // dispatch. 5000ms is random, does it matter?
        event_loop.dispatch(Duration::from_millis(5000), &mut bg)?;

        if bg.exit {
            println!("how tho");
            println!("exiting example");
            break;
        }
    }

    Ok(())
}
