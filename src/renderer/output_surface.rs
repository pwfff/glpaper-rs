use std::{sync::Arc, time::Instant};

use anyhow::Result;
use pollster::block_on;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use sctk::{
    output::OutputInfo,
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
};
use wayland_client::{protocol::wl_output::WlOutput, Connection, Proxy, QueueHandle};
use wgputoy::{context::WgpuContext, WgpuToyRenderer};

use crate::handlers::background_layer::BackgroundLayer;

/// The state of the frame callback.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCallbackState {
    /// No frame callback was requsted.
    #[default]
    None,
    /// The frame callback was requested, but not yet arrived, the redraw events are throttled.
    Requested,
    /// The callback was marked as done, and user could receive redraw requested
    Received,
}

pub struct OutputSurface {
    pub layer: Arc<LayerSurface>,

    qh: QueueHandle<BackgroundLayer>,
    frame_callback_state: FrameCallbackState,

    toy: Option<WgpuToyRenderer>,
    width: i32,
    height: i32,
    start_time: Instant,
    last_render_time: u32,
}

impl OutputSurface {
    //pub fn new(
    //    width: u32,
    //    height: u32,
    //    device: wgpu::Device,
    //    surface: wgpu::Surface,
    //    adapter: wgpu::Adapter,
    //    queue: wgpu::Queue,
    //) -> Self {
    pub(crate) async fn new(
        conn: Connection,
        qh: QueueHandle<BackgroundLayer>,
        state: &BackgroundLayer,
        output: &WlOutput,
        output_info: &OutputInfo,
    ) -> Result<Self> {
        println!("creating output surface");
        let surface = state.compositor_state.create_surface(&qh);
        let layer = state.layer_shell.create_layer_surface(
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

        let (width, height) = output_info.logical_size.unwrap();

        // Initialize wgpu
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        println!("got wgpu instance");

        /// https://github.com/rust-windowing/raw-window-handle/issues/49
        #[derive(Copy, Clone)]
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

        println!("made handle");

        let surface = unsafe { instance.create_surface(&handle).unwrap() };

        println!("made unsafe surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("couldnt get the surface");

        println!("got adapter");

        let (device, queue) = adapter
            .request_device(&Default::default(), None)
            .await
            .expect("couldnt get device");

        println!("got device and stuf..");

        let swapchain_capabilities = surface.get_capabilities(&adapter);
        let swapchain_format = swapchain_capabilities.formats[0];

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: swapchain_format,
            view_formats: vec![],
            //view_formats: vec![cap.formats[0]],
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            width: width.unsigned_abs(),
            height: height.unsigned_abs(),
            // Wayland is inherently a mailbox system.
            present_mode: wgpu::PresentMode::Mailbox,
        };
        surface.configure(&device, &surface_config);

        let ctx = WgpuContext {
            device: device.into(),
            queue: queue.into(),
            surface,
            surface_config,
        };
        let mut toy = WgpuToyRenderer::new(ctx);

        // TODO: big todo... get this stuff from the web?
        let names = vec![
            "Radius".to_string(),
            "TimeStep".to_string(),
            "Samples".to_string(),
            "AnimatedNoise".to_string(),
            "Accumulation".to_string(),
            "Exposure".to_string(),
            "BlurExponent1".to_string(),
            "BlurRadius".to_string(),
            "BlurExponent2".to_string(),
            "KerrA".to_string(),
            "KerrQ".to_string(),
            "InitSpeed".to_string(),
            "InitThick".to_string(),
            "Steps".to_string(),
            "FocalPlane".to_string(),
            "MotionBlur".to_string(),
            "Gamma".to_string(),
        ];
        let values: Vec<f32> = vec![
            1.0, 0.072, 0.218, 0.0, 1.0, 0.369, 0.393, 0.743, 0.81, 0.876, 0.0, 0.719, 0.22, 0.387,
            0.53, 0.0, 0.827,
        ];

        let names = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "DOF_Amount".to_string(),
            "DOF_Focal_Dist".to_string(),
            "Paused".to_string(),
            "D".to_string(),
        ];
        let values: Vec<f32> = vec![0.059, 0.019, 0.08, 0.882, 0.503, 0.454, 0.127];

        toy.set_custom_floats(names, values);

        let map =
            block_on(toy.preprocess_async(include_str!("./assets/fragment.default.wgsl"))).unwrap();
        //println!("{}", map.source);
        toy.compile(map);

        println!("well it compiled?");

        Ok(Self {
            qh,
            layer: layer.into(),
            toy: Some(toy),
            frame_callback_state: Default::default(),
            width,
            height,
            start_time: Instant::now(),
            last_render_time: 0,
        })
    }

    pub fn draw(&mut self) {
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.width as i32, self.height as i32);
        self.layer
            .wl_surface()
            .frame(&self.qh, self.layer.wl_surface().clone());
        self.layer.commit();
    }

    pub fn render(&mut self, time: u32) -> Result<()> {
        if self.frame_callback_state != FrameCallbackState::Received {
            return Ok(());
        }

        if time - self.last_render_time < 1000 / 10 {
            return Ok(());
        }

        match self.toy {
            Some(ref mut r) => {
                self.layer
                    .wl_surface()
                    .frame(&self.qh, self.layer.wl_surface().clone());
                self.layer
                    .wl_surface()
                    .damage_buffer(0, 0, self.width, self.height);
                self.last_render_time = time;
                //let time = self.start_time.elapsed().as_secs_f32() / 100.0;
                //r.set_time_elapsed(time);
                r.set_time_elapsed(time as f32 / 30000.);
                let frame = r.wgpu.surface.get_current_texture()?;
                if let Some(b) = r.render_to(frame) {
                    println!("o");
                    b.unmap();
                };
                self.layer.commit();
                //block_on(r.render_async());
                //r.frame_start(&mut self.surface)?;
                //r.render(&mut self.device, &mut self.queue)?;
                //r.frame_finish()
            }
            None => println!("toy went away?"),
        }

        self.frame_callback_reset();

        Ok(())
    }

    /// Get the current state of the frame callback.
    pub fn frame_callback_state(&self) -> FrameCallbackState {
        self.frame_callback_state
    }

    /// The frame callback was received, but not yet sent to the user.
    pub fn frame_callback_received(&mut self) {
        self.frame_callback_state = FrameCallbackState::Received;
    }

    /// Reset the frame callbacks state.
    pub fn frame_callback_reset(&mut self) {
        self.frame_callback_state = FrameCallbackState::None;
    }

    /// Request a frame callback if we don't have one for this window in flight.
    pub fn request_frame_callback(&mut self) {
        let surface = self.layer.wl_surface();
        match self.frame_callback_state {
            FrameCallbackState::None | FrameCallbackState::Received => {
                self.frame_callback_state = FrameCallbackState::Requested;
                surface.frame(&self.qh, surface.clone());
            }
            FrameCallbackState::Requested => (),
        }
    }
}
