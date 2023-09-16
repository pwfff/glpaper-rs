use anyhow::Result;
use pollster::block_on;
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
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

pub struct BackgroundLayer {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    pub compositor_state: Arc<CompositorState>,
    pub layer_shell: Arc<LayerShell>,

    start_time: Instant,
    oses: RefCell<HashMap<ObjectId, Arc<Mutex<OutputSurface>>>>,

    pub exit: bool,
}

impl BackgroundLayer {
    pub fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self> {
        let start_time = Instant::now();

        Ok(BackgroundLayer {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh)?.into(),
            layer_shell: LayerShell::bind(&globals, &qh)?.into(),

            start_time,
            oses: Default::default(),

            exit: false,
        })
    }

    pub fn add_toy(&mut self, os: Arc<Mutex<OutputSurface>>) {
        let id = {os.lock().unwrap().layer.wl_surface().id().clone()};
        self.oses.get_mut().insert(id, os.into());
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
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        let os = match self.oses.get_mut().get(&surface.id()) {
            Some(os) => os,
            None => return,
        };
        let mut os = os.lock().unwrap();
        os.frame_callback_received();
        os.render().unwrap();
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
        println!("configured");
        let id = &layer.wl_surface().id();
        println!("{:?}", id);
        let os = match self.oses.get_mut().get(id) {
            Some(os) => os.lock().unwrap().render(),
            None => return,
        };

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
