use std::time::Instant;
use std::{collections::HashMap, fmt::format};

use anyhow::Result;
use pollster::block_on;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use sctk::shell::{wlr_layer::LayerSurface, WaylandSurface};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};
use wgpu::{Maintain, SubmissionIndex, SurfaceTexture};
use wgputoy::{context::WgpuContext, WgpuToyRenderer};

pub struct OutputSurface {
    toy: WgpuToyRenderer,
    start_time: Instant,
    submitted_frame: Option<(SurfaceTexture, SubmissionIndex)>,

    exp: f32,
}

impl OutputSurface {
    pub(crate) async fn new(
        conn: Connection,
        layer: &LayerSurface,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        println!("creating output surface");

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
            width,
            height,
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

        //let names = vec![
        //    "A".to_string(),
        //    "B".to_string(),
        //    "C".to_string(),
        //    "DOF_Amount".to_string(),
        //    "DOF_Focal_Dist".to_string(),
        //    "Paused".to_string(),
        //    "D".to_string(),
        //];
        //let values: Vec<f32> = vec![0.059, 0.019, 0.08, 0.882, 0.503, 0.454, 0.127];

        let (names, values) = Self::custom_floats_vec(Self::custom_floats_map());
        toy.set_custom_floats(names, values);

        let map =
            block_on(toy.preprocess_async(include_str!("./assets/fragment.default.wgsl"))).unwrap();
        //println!("{}", map.source);
        toy.compile(map);

        println!("well it compiled?");

        Ok(Self {
            toy,
            start_time: Instant::now(),
            submitted_frame: None,
            exp: 0.9,
        })
    }

    fn custom_floats_map() -> Vec<(String, f32)> {
        vec![
            (format!("Radius"), 0.551),
            (format!("TimeStep"), 0.053),
            (format!("Samples"), 0.1),
            (format!("BlurRadius"), 0.489),
            (format!("VelocityDecay"), 0.018),
            (format!("Speed"), 0.197),
            (format!("BlurExponent1"), 0.621),
            (format!("BlurExponent2"), 0.),
            (format!("AnimatedNoise"), 1.),
            (format!("Accumulation"), 0.962),
            (format!("Exposure"), 0.224),
        ]
    }

    fn custom_floats_vec(fs: Vec<(String, f32)>) -> (Vec<String>, Vec<f32>) {
            fs.iter().fold((vec![], vec![]), |(mut ks, mut vs), (k, v)| {
                ks.push(k.clone());
                vs.push(*v);
                (ks, vs)
            })
    }

    pub fn set_fft(&mut self, med_fv: f32, max_fv: f32) {
        let mut fs = Self::custom_floats_map();
        self.exp = med_fv.max(0.1).max(self.exp) * 0.9;
        for kv in fs.iter_mut() {
            if kv.0 == "BlurRadius" {
                kv.1 = self.exp;
            }
        }
        let (names, values) = Self::custom_floats_vec(fs);
        self.toy.set_custom_floats(names, values)
    }

    pub fn draw(&mut self) -> Result<()> {
        self.toy.wgpu.device.poll(Maintain::Poll);

        if self.submitted_frame.is_some() {
            //println!("already got one hun");
            return Ok(());
        }

        let time = self.start_time.elapsed().as_micros();
        //r.set_time_elapsed(time);
        self.toy.set_time_elapsed(time as f32 / 100.);
        let frame = self.toy.wgpu.surface.get_current_texture()?;
        let (_, submitted) = self.toy.render_to(frame);
        self.submitted_frame = Some(submitted);

        self.toy.wgpu.device.poll(Maintain::Poll);

        Ok(())
    }

    pub fn wait(&mut self) -> Result<()> {
        if let Some((_, i)) = &self.submitted_frame {
            self.toy
                .wgpu
                .device
                .poll(Maintain::WaitForSubmissionIndex(i.clone()));
        }

        Ok(())
    }

    pub fn render(&mut self, layer: &WlSurface) -> Result<()> {
        if let Some((frame, i)) = self.submitted_frame.take() {
            self.toy
                .wgpu
                .device
                .poll(Maintain::WaitForSubmissionIndex(i));
            frame.present();
        }
        layer.commit();

        Ok(())
    }
}
