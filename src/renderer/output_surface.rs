use anyhow::{anyhow, Result};
use sctk::{
    output::OutputInfo,
    shell::{wlr_layer::LayerSurface, WaylandSurface},
};
use wayland_client::Proxy;
use wgpu::{ShaderModule, ShaderModuleDescriptor};

use super::renderable::{RenderConfig, RenderState, Renderable};

pub struct OutputSurface {
    output_info: OutputInfo,

    layer: LayerSurface,

    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface,

    renderable: Option<Renderable>,
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
        OutputSurface {
            output_info,
            layer,
            device,
            surface,
            adapter,
            queue,
            renderable: None,
        }
    }

    pub fn create_shader_module(&self, desc: ShaderModuleDescriptor) -> ShaderModule {
        self.device.create_shader_module(desc)
    }

    fn logical_size(&self) -> Result<(u32, u32)> {
        let (width, height) = self.output_info.logical_size.ok_or(anyhow!("illogical"))?;
        Ok((width.unsigned_abs(), height.unsigned_abs()))
    }

    pub fn layer_matches(&self, layer: &LayerSurface) -> bool {
        self.layer.wl_surface().id() == layer.wl_surface().id()
    }

    pub fn render(&mut self) -> Result<()> {
        match self.renderable {
            Some(ref mut r) => {
                r.frame_start(&mut self.surface)?;
                r.render(&mut self.device, &mut self.queue)?;
                r.frame_finish()
            },
            None => Ok(()),
        }
    }

    pub fn prep_render_pipeline(&mut self, config: &RenderConfig) -> Result<()> {
        let swapchain_capabilities = self.surface.get_capabilities(&self.adapter);
        let swapchain_format = swapchain_capabilities.formats[0];

        let frag_state = wgpu::FragmentState {
            module: &config.frag_shader,
            entry_point: "main",
            targets: &[Some(swapchain_format.into())],
        };

        let vert_state = wgpu::VertexState {
            module: &config.vert_shader,
            entry_point: "main",
            buffers: &[],
        };

        let render_state = RenderState::new(&self.device, &self.output_info);

        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&render_state.uniform_bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: vert_state,
                fragment: Some(frag_state),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

        let (width, height) = self.logical_size()?;
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

        self.surface.configure(&self.device, &surface_config);

        self.renderable = Some(Renderable::new(pipeline, surface_config, render_state)?);

        Ok(())
    }
}
