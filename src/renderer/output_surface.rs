use anyhow::{Result};
use pollster::block_on;
use sctk::{
    output::OutputInfo,
    shell::{wlr_layer::LayerSurface, WaylandSurface},
};
use wayland_client::Proxy;
use wgputoy::{context::WgpuContext, WgpuToyRenderer};

pub struct OutputSurface {
    layer: LayerSurface,
    pub toy: Option<WgpuToyRenderer>,
}

impl OutputSurface {
    pub fn new(
        output_info: OutputInfo,
        layer: LayerSurface,
        device: wgpu::Device,
        surface: wgpu::Surface,
        adapter: wgpu::Adapter,
        queue: wgpu::Queue,
    ) -> Self {
        let swapchain_capabilities = surface.get_capabilities(&adapter);
        let swapchain_format = swapchain_capabilities.formats[0];

        let (width, height) = output_info.logical_size.unwrap();

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
            queue,
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
        toy.set_custom_floats(names, values);

        let map =
            block_on(toy.preprocess_async(include_str!("./assets/fragment.default.wgsl"))).unwrap();
        println!("{}", map.source);
        toy.compile(map);

        OutputSurface {
            layer,
            toy: Some(toy),
        }
    }

    pub fn layer_matches(&self, layer: &LayerSurface) -> bool {
        self.layer.wl_surface().id() == layer.wl_surface().id()
    }

    pub fn render(&mut self) -> Result<()> {
        match self.toy {
            Some(ref mut r) => {
                block_on(r.render_async());
                //r.frame_start(&mut self.surface)?;
                //r.render(&mut self.device, &mut self.queue)?;
                //r.frame_finish()
                Ok(())
            }
            None => Ok(()),
        }
    }
}
