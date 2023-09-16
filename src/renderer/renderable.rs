use std::time::Instant;

use anyhow::{bail, Result};
use sctk::output::OutputInfo;
use wgpu::{
    util::DeviceExt, BindGroup, BindGroupLayout, Buffer, Device, Queue, RenderPipeline,
    ShaderModule, Surface, SurfaceConfiguration, SurfaceTexture, TextureView,
};

use super::output_surface::OutputSurface;

const UNIFORM_GROUP_ID: u32 = 0;

const VERT: &'static str = include_str!("./assets/vertex.wgsl");
const FRAG_PREFIX: &'static str = include_str!("./assets/fragment.prefix.wgsl");
const FRAG_SUFFIX: &'static str = include_str!("./assets/fragment.suffix.wgsl");

pub struct RenderConfig {
    pub frag_shader: ShaderModule,
    pub vert_shader: ShaderModule,
}

impl RenderConfig {
    pub fn new(output_surface: &OutputSurface, shader_source: &str) -> Result<Self> {
        let mut frag_shader_source =
            String::with_capacity(FRAG_PREFIX.len() + shader_source.len() + FRAG_SUFFIX.len());
        frag_shader_source.push_str(FRAG_PREFIX);
        frag_shader_source.push_str(shader_source);
        frag_shader_source.push_str(FRAG_SUFFIX);

        let frag_shader = output_surface.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fragment_shader"),
            source: wgpu::ShaderSource::Wgsl(frag_shader_source.into()),
        });

        let vert_shader = output_surface.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vertex_shader"),
            source: wgpu::ShaderSource::Wgsl(VERT.into()),
        });

        Ok(Self {
            frag_shader,
            vert_shader,
        })
    }
}

pub struct Renderable {
    pipeline: RenderPipeline,

    surface_configuration: SurfaceConfiguration,
    render_state: RenderState,

    surface_texture: Option<SurfaceTexture>,
    texture_view: Option<TextureView>,
}

impl Renderable {
    pub fn new(
        pipeline: RenderPipeline,
        surface_configuration: SurfaceConfiguration,
        render_state: RenderState,
    ) -> Result<Self> {
        Ok(Self {
            pipeline,
            surface_configuration,
            render_state,
            surface_texture: None,
            texture_view: None,
        })
    }

    pub fn frame_start(&mut self, surface: &mut Surface) -> Result<()> {
        if self.surface_texture.is_some() {
            bail!("Non-finished wgpu::SurfaceTexture found.")
        }

        let surface_texture = surface.get_current_texture().expect("couldnt get texture");

        self.surface_texture = Some(surface_texture);

        if let Some(surface_texture) = &self.surface_texture {
            self.texture_view = Some(surface_texture.texture.create_view(
                &wgpu::TextureViewDescriptor {
                    format: Some(self.surface_configuration.format),
                    ..wgpu::TextureViewDescriptor::default()
                },
            ));
        }

        Ok(())
    }

    pub fn render(&mut self, device: &mut Device, queue: &mut Queue) -> Result<()> {
        if self.texture_view.is_none() {
            bail!("No actived wgpu::TextureView found.")
        }

        let view = self.texture_view.as_ref().unwrap();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        self.render_state.update_time();

        queue.write_buffer(
            &self.render_state.uniform_buffer,
            0,
            self.render_state.as_bytes(),
        );

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });

            // TODO: is this important...?
            //if let Some(viewport) = &self.output_surface.viewport {
            //    render_pass.set_viewport(
            //        viewport.x,
            //        viewport.y,
            //        viewport.width,
            //        viewport.height,
            //        viewport.min_depth,
            //        viewport.max_depth,
            //    );
            //}

            render_pass.set_pipeline(&self.pipeline);

            render_pass.set_bind_group(
                UNIFORM_GROUP_ID,
                &self.render_state.uniform_bind_group,
                &[],
            );

            //let mut index = 1;
            //for (_, bind_group) in &self.texture_bind_groups {
            //    render_pass.set_bind_group(index, bind_group, &[]);
            //    index += 1;
            //}

            render_pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));

        Ok(())
    }

    pub fn frame_finish(&mut self) -> Result<()> {
        if self.surface_texture.is_none() {
            bail!("No actived wgpu::SurfaceTexture found.")
        }

        if let Some(surface_texture) = self.surface_texture.take() {
            surface_texture.present();
        }

        Ok(())
    }
}

pub struct RenderState {
    time_instant: Instant,

    uniform_bind_group: BindGroup,
    // TODO: does this need to be public...?
    pub uniform_bind_group_layout: BindGroupLayout,

    uniform: Uniform,
    uniform_buffer: Buffer,
}

impl RenderState {
    pub fn new(device: &Device, output_info: &OutputInfo) -> Self {
        let mut uniform = Uniform::default();

        let (width, height) = output_info.logical_size.expect("illogical size?");
        uniform.resolution = [width as f32, height as f32];

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: uniform.as_bytes(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Uniform Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uniform Bind Group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let time_instant = Instant::now();

        Self {
            time_instant,
            uniform_bind_group,
            uniform_bind_group_layout,
            uniform,
            uniform_buffer,
        }
    }

    pub fn update_time(&mut self) {
        self.uniform.time = self.time_instant.elapsed().as_secs_f32();
    }

    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(&self.uniform)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniform {
    pub cursor: [f32; 2],
    pub mouse_down: u32,
    _padding0: u32,
    pub mouse_press: [f32; 2],
    pub mouse_release: [f32; 2],
    pub resolution: [f32; 2],
    pub time: f32,
    _padding1: u32,
}

impl Uniform {
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}
