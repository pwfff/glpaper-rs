use anyhow::Result;
use std::{collections::HashMap, sync::Arc, time::Instant};
use wayland_backend::client::ObjectId;

use crate::renderer::output_surface::OutputSurface;

use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_seat,
    output::{OutputHandler, OutputInfo, OutputState},
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
    globals::GlobalList,
    protocol::{wl_output, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};

/// https://github.com/rust-windowing/raw-window-handle/issues/49
#[derive(Copy, Clone)]
struct YesRawWindowHandleImplementingHasRawWindowHandleIsUnsound(RawDisplayHandle, RawWindowHandle);

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

pub struct BackgroundLayer {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,

    start_time: Instant,
    oses: HashMap<ObjectId, OutputSurface>,

    pub exit: bool,
}

impl BackgroundLayer {
    pub fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self> {
        let start_time = Instant::now();

        Ok(BackgroundLayer {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh)?,
            layer_shell: LayerShell::bind(&globals, &qh)?,

            start_time,
            oses: HashMap::new(),

            exit: false,
        })
    }

    pub async fn render(&mut self) -> Result<()> {
        let time = self.start_time.elapsed().as_secs_f32() / 10.0;
        let mut handles = vec![];
        for os in self.oses.values_mut() {
            match os.toy.as_mut() {
                Some(toy) => {
                    toy.set_time_elapsed(time);
                    handles.push(toy.render_async());
                    //match toy.wgpu.surface.get_current_texture() {
                    //    Ok(f) => {
                    //        let buf = toy.render_to(f);

                    //        println!("ididit");
                    //    }
                    //    Err(e) => {
                    //        println!("{:?}", e)
                    //    }
                    //};
                }
                None => {}
            }
        }
        futures::future::join_all(handles).await;
        Ok(())
    }
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
        println!("frame");
        //self.render().unwrap();
    }
}

impl LayerShellHandler for BackgroundLayer {
    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        c: LayerSurfaceConfigure,
        _: u32,
    ) {
        let id = &layer.wl_surface().id();

        if self.oses.contains_key(id) {
            return;
        };

        println!("configuring");
        let (width, height) = c.new_size;

        println!("ughhghguhguhg");
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

            YesRawWindowHandleImplementingHasRawWindowHandleIsUnsound(display_handle, window_handle)
        };

        let surface = unsafe { instance.create_surface(&handle).unwrap() };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("couldnt get the surface");

        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None))
            .expect("couldnt get device");

        println!("got device and stuf..");

        self.oses.insert(
            id.clone(),
            OutputSurface::new(width, height, device, surface, adapter, queue),
        );

        //layer.wl_surface().frame(qh, layer.wl_surface().clone());
        //for output_surface in self.output_surfaces.iter_mut() {
        //    if !output_surface.layer_matches(this_layer) {
        //        continue;
        //    }

        //    // TODO: what was this for
        //    //let cap = output_surface
        //    //    .surface
        //    //    .get_capabilities(&output_surface.adapter);

        //    // TODO: pull this crap out? change it on the fly? how do we integrate real time audio
        //    // into the uniforms?
        //    //let config = RenderConfig::new(
        //    //    output_surface,
        //    //    include_str!("./renderer/assets/fragment.default.wgsl"),
        //    //)
        //    //.unwrap();

        //    //output_surface.prep_render_pipeline(config).unwrap();
        //    output_surface.render().unwrap();
        //}
    }

    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        todo!()
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
