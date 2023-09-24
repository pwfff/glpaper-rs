use anyhow::Result;
use pollster::block_on;
use std::sync::Arc;
use wayland_backend::client::ObjectId;

use crate::renderer::output_surface::OutputSurface;
use sctk::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
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
        wl_surface,
    },
    Connection, Proxy, QueueHandle,
};

pub struct Background {
    output: WlOutput,
    output_info: OutputInfo,

    layer_surface: LayerSurface,

    renderer: Option<OutputSurface>,
}

trait Backgrounds {
    fn by_id(&mut self, id: &ObjectId) -> Option<&mut Background>;
    fn by_output(&mut self, output: &WlOutput) -> Option<&mut Background>;
}

impl Backgrounds for Vec<Background> {
    fn by_id(&mut self, id: &ObjectId) -> Option<&mut Background> {
        self.iter_mut()
            .find(|b| &b.layer_surface.wl_surface().id() == id)
    }

    fn by_output(&mut self, output: &WlOutput) -> Option<&mut Background> {
        self.iter_mut().find(|b| &b.output == output)
    }
}

pub struct BackgroundLayer {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: Arc<CompositorState>,
    layer_shell: Arc<LayerShell>,

    backgrounds: Vec<Background>,

    pub exit: bool,
    shader_id: Option<String>,
}

impl BackgroundLayer {
    pub fn new(
        globals: &GlobalList,
        shader_id: Option<String>,
        qh: &QueueHandle<Self>,
    ) -> Result<Self> {
        Ok(BackgroundLayer {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            compositor_state: CompositorState::bind(&globals, &qh)?.into(),
            layer_shell: LayerShell::bind(&globals, &qh)?.into(),
            shader_id,

            backgrounds: vec![],

            exit: false,
        })
    }

    pub fn draw(&mut self) {
        for b in self.backgrounds.iter_mut() {
            if let Some(ref mut r) = b.renderer {
                r.draw().unwrap()
            }
        }
    }

    //pub fn render(&mut self) {
    //    match &mut self.os {
    //        Some(os) => os.render().unwrap(),
    //        None => return,
    //    };
    //}

    // TODO: put in config struct
    pub fn configure_output(
        &mut self,
        qh: &QueueHandle<Self>,
        output: WlOutput,
        output_info: OutputInfo,
    ) {
        if self.backgrounds.by_output(&output).is_some() {
            // TODO: pass along config info, reset
            return;
        }

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

        self.backgrounds.push(Background {
            output,
            output_info,
            layer_surface: layer,
            renderer: None,
        });
    }

    pub fn reset(&mut self) -> Result<()> {
        // TODO: reset all, reset by id, just use configure output??
        //if let Some(ref mut os) = self.backgrounds.by_id(id) {
        //    return os.reset();
        //}

        Ok(())
    }

    pub fn set_fft(&mut self, max_f: f32, max_fv: f32) {
        for b in self.backgrounds.iter_mut() {
            if let Some(ref mut os) = b.renderer {
                os.set_fft(max_f, max_fv);
            }
        }
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
        surface.frame(_qh, surface.clone());
        if let Some(b) = self.backgrounds.by_id(&surface.id()) {
            if let Some(ref mut os) = b.renderer {
                os.render(surface).unwrap();
            }
        }
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
        let (width, height) = c.new_size;
        let surface = layer.wl_surface();
        match self.backgrounds.by_id(&surface.id()) {
            Some(ref mut b) => match b.renderer {
                Some(ref mut os) => {
                    os.draw().unwrap();
                }
                None => {
                    let mut os = block_on(OutputSurface::new(
                        conn.clone(),
                        layer,
                        width,
                        height,
                        self.shader_id.clone(),
                    ))
                    .unwrap();
                    surface.frame(qh, surface.clone());
                    os.draw().unwrap();
                    os.render(surface).unwrap();
                    b.renderer = Some(os);
                }
            },
            None => {}
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

delegate_compositor!(BackgroundLayer);
delegate_output!(BackgroundLayer);

//delegate_seat!(BackgroundLayer);

delegate_layer!(BackgroundLayer);

delegate_registry!(BackgroundLayer);

impl ProvidesRegistryState for BackgroundLayer {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
