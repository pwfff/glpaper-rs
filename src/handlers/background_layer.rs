use anyhow::Result;
use pollster::block_on;
use std::sync::Arc;

use crate::renderer::output_surface::OutputSurface;
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
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
    protocol::{
        wl_output::{self, WlOutput},
        wl_seat, wl_surface,
    },
    Connection, QueueHandle,
};

pub struct BackgroundLayer {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: Arc<CompositorState>,
    layer_shell: Arc<LayerShell>,
    layer_surface: Option<LayerSurface>,

    os: Option<OutputSurface>,

    pub exit: bool,
}

impl BackgroundLayer {
    pub fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self> {
        Ok(BackgroundLayer {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh)?.into(),
            layer_shell: LayerShell::bind(&globals, &qh)?.into(),

            os: None,
            layer_surface: None,

            exit: false,
        })
    }

    pub fn draw(&mut self) {
        match &mut self.os {
            Some(os) => os.draw().unwrap(),
            None => return,
        };
    }

    pub fn render(&mut self) {
        match &mut self.os {
            Some(os) => os.render().unwrap(),
            None => return,
        };
    }

    pub fn create_layer(&mut self, qh: &QueueHandle<Self>, output: WlOutput) {
        println!("creating layer");
        let surface = self.compositor_state.create_surface(&qh);
        let layer = self.layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Background,
            Some(""),
            Some(&output),
        );
        //layer.set_size(width.unsigned_abs(), height.unsigned_abs());
        layer.set_anchor(Anchor::all());
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        self.layer_surface = Some(layer);
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
        _: u32,
    ) {
        self.render();
        surface.frame(_qh, surface.clone());
        surface.commit();
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
        println!("configure");
        let (width, height) = c.new_size;
        if self.os.is_none() {
            let mut os = block_on(OutputSurface::new(
                conn.clone(),
                layer,
                width,
                height,
            ))
            .unwrap();
            layer.wl_surface().frame(qh, layer.wl_surface().clone());
            os.draw().unwrap();
            os.render().unwrap();
            println!("did first draw");
            self.os = Some(os);
        }
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
