use std::{time::Duration, io};

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

    let compositor_state = CompositorState::bind(&globals, &qh).or_else(|_| Err(anyhow!("uhh")))?;
    let layer_shell = LayerShell::bind(&globals, &qh).or_else(|_| Err(anyhow!("uhh")))?;

    let layers: Vec<LayerSurface> = bg
        .output_state()
        .outputs()
        .map(|output| {
            //let (width, height) = bg.output_state().info(&output).unwrap().logical_size.unwrap();

            let surface = compositor_state.create_surface(&qh);
            let layer = layer_shell.create_layer_surface(
                &qh,
                surface,
                Layer::Background,
                Some("glpaper-rs"),
                Some(&output),
            );
            //layer.set_size(width.unsigned_abs(), height.unsigned_abs());
            layer.set_anchor(Anchor::all());
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();

            layer
        })
        .collect();

    // dispatch once to get everything set up. probably unnecessary?
    //event_queue.blocking_dispatch(&mut background_layer)?;
    event_queue.roundtrip(&mut bg).unwrap();

    let mut event_loop: EventLoop<BackgroundLayer> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");
    let loop_handle = event_loop.handle();
    WaylandSource::new(event_queue)
        .unwrap()
        .insert(loop_handle)
        .unwrap();

    // TODO: this seems wrong...
    let mut ugh = tokio::time::interval(Duration::from_millis(1000/10));
    loop {
        //event_queue.dispatch_pending(&mut bg).unwrap();
        //event_queue.blocking_dispatch(&mut bg).unwrap();

        event_loop.dispatch(Duration::from_millis(1), &mut bg)?;
        ugh.tick().await;
        bg.render().await.unwrap();
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
    }

    for layer in layers {
        drop(layer)
    }

    //for output_surface in output_surfaces.into_iter() {
    // TODO: do i still need this? am i dropping the right thing?
    //drop(output_surface);
    //drop(output_surface.surface);
    //drop(output_surface.layer);
    //}

    Ok(())
}
