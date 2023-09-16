use std::{io, time::{Duration, Instant}, sync::{Arc, Mutex}, thread};

use anyhow::{anyhow, Result};

use handlers::background_layer::BackgroundLayer;
use sctk::{
    compositor::CompositorState,
    output::OutputHandler,
    reexports::calloop::EventLoop,
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

    let mut oses = vec![];
    for output in bg.output_state().outputs() {
        let output_info = bg.output_state().info(&output).unwrap();
        let os = OutputSurface::new(conn.clone(), qh.clone(), &bg, &output, &output_info).await?;
        let arctex: Arc<Mutex<OutputSurface>> = Arc::new(os.into());
        bg.add_toy(arctex.clone());
        oses.push(arctex);
    }

    //let mut event_loop: EventLoop<BackgroundLayer> =
    //    EventLoop::try_new().expect("Failed to initialize the event loop!");
    //let loop_handle = event_loop.handle();
    //WaylandSource::new(event_queue)
    //    .unwrap()
    //    .insert(loop_handle)
    //    .unwrap();

    // TODO: this seems wrong...
    //let mut ugh = tokio::time::interval(Duration::from_millis(1000/10));
    let start = Instant::now();
    loop {
        for os in oses.iter_mut() {
            let mut os = os.lock().unwrap();
            os.render(start.elapsed().as_millis() as u32).unwrap();
        }

        event_queue.dispatch_pending(&mut bg).unwrap();
        //event_queue.blocking_dispatch(&mut bg)?;
        //event_queue.flush()?;

        //event_loop.dispatch(Duration::from_millis(100), &mut bg)?;

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
