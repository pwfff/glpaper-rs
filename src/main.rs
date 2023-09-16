use std::{
    io,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};

use handlers::background_layer::BackgroundLayer;
use sctk::{
    compositor::CompositorState,
    output::OutputHandler,
    reexports::calloop::{EventLoop, timer::{TimeoutAction, Timer}},
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
        WaylandSurface,
    },
};
use wayland_client::{globals::registry_queue_init, Connection, WaylandSource};

use crate::renderer::output_surface::OutputSurface;

mod handlers;
mod renderer;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // first get connection to wayland
    let conn = Connection::connect_to_env().unwrap();

    // now set up main handler
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    //let output_state = OutputState::new(&globals, &qh);

    // construct background_layer, then event loop so we can trigger rendering over time without depending on
    // messages coming in from wayland
    // TODO: kick this stuff off in two separate threads(?) instead of depending on the dispatch
    // timeout
    let mut bg = BackgroundLayer::new(&globals, &qh)?;
    println!("hui");

    // dispatch once to get everything set up. probably unnecessary?
    //event_queue.blocking_dispatch(&mut background_layer)?;
    event_queue.roundtrip(&mut bg).unwrap();

    let pattern = std::env::args().nth(1).expect("no display given");

    let os = match bg.output_state().outputs().find_map(|output| {
        let output_info = bg.output_state().info(&output).unwrap();
        if output_info.clone().name.unwrap() != pattern {
            None
        } else {
            Some((output, output_info))
        }
    }) {
        Some((output, output_info)) => {
            OutputSurface::new(conn.clone(), qh.clone(), &bg, &output, &output_info)
                .await
                .unwrap()
        }
        None => return Err(anyhow!("couldn't find display")),
    };

    bg.add_toy(Arc::new(Mutex::new(os)));

    let mut event_loop: EventLoop<BackgroundLayer> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");
    let loop_handle = event_loop.handle();

    let start = Instant::now();
    let mut last_frame = Instant::now();
    const fps: f32 = 20.;
    const mspf: f32 = 1000. / fps;
    let mspf_d = Duration::from_millis(mspf as u64);

    let t = Timer::from_duration(mspf_d);

    loop_handle
        .insert_source(t, move |e, meta, bg| {
            //bg.render(start.elapsed().as_millis() as u32);
            bg.want_frame();
            bg.request_callback();
            TimeoutAction::ToDuration(mspf_d)
        })
        .unwrap();

    let ws = WaylandSource::new(event_queue).unwrap();
    ws.insert(loop_handle).unwrap();

    loop {
        //bg.render(start.elapsed().as_millis() as u32);

        //event_queue.dispatch_pending(&mut bg).unwrap();
        //event_queue.blocking_dispatch(&mut bg)?;
        //event_queue.flush()?;

        event_loop.dispatch(Duration::from_millis(1), &mut bg)?;

        //for os in oses.iter_mut() {
        //    let mut os = os.lock().unwrap();
        //    os.render();
        //    //os.request_frame_callback();
        //    //os.render()?;
        //}
        //    //bg.render();
        //    //let time = start_time.elapsed().as_secs_f32() / 100.0;

        //    //for os in bg.output_surfaces.iter_mut() {
        //    //    match os.toy.as_mut() {
        //    //        Some(toy) => {
        //    //            sender.send(toy);
        //    //            //toy.set_time_elapsed(time);
        //    //            //pollster::block_on(toy.render_async());
        //    //        }
        //    //        None => {}
        //    //    }
        //    //}
        //})?;
        //event_queue.blocking_dispatch(&mut background_layer).unwrap();

        if bg.exit {
            println!("how tho");
            println!("exiting example");
            break;
        }

        thread::sleep(Duration::from_millis(10));
    }

    //for os in oses {
    //    drop(os.surface)
    //}

    //for output_surface in output_surfaces.into_iter() {
    // TODO: do i still need this? am i dropping the right thing?
    //drop(output_surface);
    //drop(output_surface.surface);
    //drop(output_surface.layer);
    //}

    Ok(())
}
