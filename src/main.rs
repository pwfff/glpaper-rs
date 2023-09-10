use std::time::Duration;

use anyhow::Result;

use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use renderer::{output_surface::OutputSurface, renderable::RenderConfig};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle, WaylandSource,
};

mod handlers;
mod renderer;

use crate::handlers::list_outputs::ListOutputs;

fn main() -> Result<()> {
    env_logger::init();

    // first get connection to wayland
    let conn = Connection::connect_to_env().unwrap();

    // set up output listing handler
    // TODO: can we combine this with our existing handler? does it leak anything when we just
    // leave it here...?
    let mut list_outputs = ListOutputs::new(&conn)?;
    let outputs = list_outputs.output_state();

    // now set up main handler
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let compositor_state = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;

    let output_surfaces: Vec<OutputSurface> = outputs.outputs().map(|output| {
        let surface = compositor_state.create_surface(&qh);
        let layer =
            layer_shell.create_layer_surface(&qh, surface, Layer::Background, Some("glpaper-rs"), Some(&output));
        layer.set_size(123, 123);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        // Initialize wgpu
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Create the raw window handle for the surface.
        let handle = {
            let mut handle = WaylandDisplayHandle::empty();
            handle.display = conn.backend().display_ptr() as *mut _;
            let display_handle = RawDisplayHandle::Wayland(handle);

            let mut handle = WaylandWindowHandle::empty();
            handle.surface = layer.wl_surface().id().as_ptr() as *mut _;
            let window_handle = RawWindowHandle::Wayland(handle);

            /// https://github.com/rust-windowing/raw-window-handle/issues/49
            struct YesRawWindowHandleImplementingHasRawWindowHandleIsUnsound(
                RawDisplayHandle,
                RawWindowHandle,
            );

            unsafe impl HasRawDisplayHandle for YesRawWindowHandleImplementingHasRawWindowHandleIsUnsound {
                fn raw_display_handle(&self) -> RawDisplayHandle {
                    self.0
                }
            }

            unsafe impl HasRawWindowHandle for YesRawWindowHandleImplementingHasRawWindowHandleIsUnsound {
                fn raw_window_handle(&self) -> RawWindowHandle {
                    self.1
                }
            }

            YesRawWindowHandleImplementingHasRawWindowHandleIsUnsound(display_handle, window_handle)
        };

        let surface = unsafe { instance.create_surface(&handle).unwrap() };

        // Pick a supported adapter
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("couldnt get the surface");

        let output_info = outputs.info(&output).expect("output has no info");

        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).expect("couldnt get device");

        OutputSurface::new(
            output_info,
            layer,
            device,
            surface,
            adapter,
            queue,
        )
    }).collect();

    // construct background_layer, then event loop so we can trigger rendering over time without depending on
    // messages coming in from wayland
    // TODO: kick this stuff off in two separate threads(?) instead of depending on the dispatch
    // timeout
    let mut background_layer = BackgroundLayer {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),

        exit: false,
        output_surfaces,
    };

    // dispatch once to get everything set up. probably unnecessary?
    event_queue.blocking_dispatch(&mut background_layer)?;

    let mut event_loop: EventLoop<BackgroundLayer> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");
    let loop_handle = event_loop.handle();
    WaylandSource::new(event_queue)
        .unwrap()
        .insert(loop_handle)
        .unwrap();

    // We don't draw immediately, the configure will notify us when to first draw.
    loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut background_layer)
            .unwrap();
        //event_queue.blocking_dispatch(&mut background_layer).unwrap();

        for os in background_layer.output_surfaces.iter_mut() {
            match os.render() {
                Ok(_) => {}
                Err(e) => {
                    println!("{}", e)
                }
            };
        }

        if background_layer.exit {
            println!("exiting example");
            break;
        }
    }

    for output_surface in background_layer.output_surfaces.into_iter() {
        drop(output_surface);
        //drop(output_surface.surface);
        //drop(output_surface.layer);
    }

    Ok(())
}

struct BackgroundLayer {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,

    exit: bool,

    output_surfaces: Vec<OutputSurface>,
}

impl CompositorHandler for BackgroundLayer {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Not needed for this example.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
}

impl OutputHandler for BackgroundLayer {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for BackgroundLayer {
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        this_layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        for output_surface in self.output_surfaces.iter_mut() {
            if !output_surface.layer_matches(this_layer) {
                continue;
            }

            // TODO: what was this for
            //let cap = output_surface
            //    .surface
            //    .get_capabilities(&output_surface.adapter);

            let config = RenderConfig::new(
                output_surface,
                "fn main_image(frag_color: vec4<f32>, frag_coord: vec2<f32>) -> vec4<f32> {
    let uv = frag_coord / u.resolution;
    let color = 0.5 + 0.5 * cos(u.time + uv.xyx + vec3(0.0, 2.0, 4.0));
    return vec4(color, 1.0);
}",
            )
            .unwrap();

            output_surface.prep_render_pipeline(&config).unwrap();
            output_surface.render().unwrap();
        }
    }

    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        todo!()
    }
}

impl SeatHandler for BackgroundLayer {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

delegate_compositor!(BackgroundLayer);
delegate_output!(BackgroundLayer);

delegate_seat!(BackgroundLayer);

delegate_layer!(BackgroundLayer);

delegate_registry!(BackgroundLayer);

impl ProvidesRegistryState for BackgroundLayer {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
