use std::{borrow::{Cow, BorrowMut}, time::Instant};

use anyhow::{anyhow, bail, Result};

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
    globals::registry_queue_init,
    protocol::{wl_output, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};
use wgpu::{
    util::DeviceExt, BindGroup, Buffer, RenderPipeline, SurfaceConfiguration, SurfaceTexture,
    TextureView,
};

const UNIFORM_GROUP_ID: u32 = 0;

fn main() -> Result<()> {
    env_logger::init();

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let mut list_outputs = ListOutputs {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
    };

    // `OutputState::new()` binds the output globals found in `registry_queue_init()`.
    //
    // After the globals are bound, we need to dispatch again so that events may be sent to the newly
    // created objects.
    event_queue.roundtrip(&mut list_outputs)?;

    // Now our outputs have been initialized with data, we may access what outputs exist and information about
    // said outputs using the output delegate.
    for output in list_outputs.output_state.outputs() {
        print_output(
            &list_outputs
                .output_state
                .info(&output)
                .ok_or_else(|| anyhow!("output has no info"))?,
        );
    }

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let compositor_state = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;

    let output_surfaces: Vec<OutputSurface> = list_outputs.output_state.outputs().map(|output| {
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

        let output_info = list_outputs.output_state.info(&output).expect("output has no info");

        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).expect("couldnt get device");
        OutputSurface {
            output_info,
            layer,
            device,
            surface,
            adapter,
            queue,
            renderable: None,
        }
    }).collect();

    let mut wgpu = Wgpu {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),

        exit: false,
        output_surfaces,
    };

    // We don't draw immediately, the configure will notify us when to first draw.
    loop {
        event_queue.blocking_dispatch(&mut wgpu).unwrap();

        if wgpu.exit {
            println!("exiting example");
            break;
        }
    }

    for output_surface in wgpu.output_surfaces.into_iter() {
        drop(output_surface.surface);
        drop(output_surface.layer);
    }

    Ok(())
}

struct ListOutputs {
    registry_state: RegistryState,
    output_state: OutputState,
}

// In order to use OutputDelegate, we must implement this trait to indicate when something has happened to an
// output and to provide an instance of the output state to the delegate when dispatching events.
impl OutputHandler for ListOutputs {
    // First we need to provide a way to access the delegate.
    //
    // This is needed because delegate implementations for handling events use the application data type in
    // their function signatures. This allows the implementation to access an instance of the type.
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    // Then there exist these functions that indicate the lifecycle of an output.
    // These will be called as appropriate by the delegate implementation.

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

// Now we need to say we are delegating the responsibility of output related events for our application data
// type to the requisite delegate.
delegate_output!(ListOutputs);

// In order for our delegate to know of the existence of globals, we need to implement registry
// handling for the program. This trait will forward events to the RegistryHandler trait
// implementations.
delegate_registry!(ListOutputs);

fn print_output(info: &OutputInfo) {
    println!("{}", info.model);

    if let Some(name) = info.name.as_ref() {
        println!("\tname: {name}");
    }

    if let Some(description) = info.description.as_ref() {
        println!("\tdescription: {description}");
    }

    println!("\tmake: {}", info.make);
    println!("\tx: {}, y: {}", info.location.0, info.location.1);
    println!("\tsubpixel: {:?}", info.subpixel);
    println!(
        "\tphysical_size: {}×{}mm",
        info.physical_size.0, info.physical_size.1
    );
    if let Some((x, y)) = info.logical_position.as_ref() {
        println!("\tlogical x: {x}, y: {y}");
    }
    if let Some((width, height)) = info.logical_size.as_ref() {
        println!("\tlogical width: {width}, height: {height}");
    }
    println!("\tmodes:");

    for mode in &info.modes {
        println!("\t\t{mode}");
    }
}

// In order for delegate_registry to work, our application data type needs to provide a way for the
// implementation to access the registry state.
//
// We also need to indicate which delegates will get told about globals being created. We specify
// the types of the delegates inside the array.
impl ProvidesRegistryState for ListOutputs {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers! {
        // Here we specify that OutputState needs to receive events regarding the creation and destruction of
        // globals.
        OutputState,
    }
}

struct OutputSurface {
    output_info: OutputInfo,

    layer: LayerSurface,

    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface,

    renderable: Option<Renderable>,
}

struct Wgpu {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,

    exit: bool,

    output_surfaces: Vec<OutputSurface>,
}

impl CompositorHandler for Wgpu {
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
        for output_surface in self.output_surfaces.iter() {
        }
    }
}

impl OutputSurface {
    fn render(&mut self) -> Result<()> {
        match self.renderable.as_mut() {
            Some(ref mut r) => {
                r.frame_start(self)?;
                r.render(self)?;
                r.frame_finish()?;
            },
            None => {},
        };

        Ok(())
    }
}

impl OutputHandler for Wgpu {
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

impl LayerShellHandler for Wgpu {
    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        this_layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        for output_surface in self.output_surfaces.iter_mut() {
            if output_surface.layer.wl_surface().id() != this_layer.wl_surface().id() {
                continue;
            }

            let cap = output_surface.surface.get_capabilities(&output_surface.adapter);

            let shader = output_surface.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fragment_shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                    "struct Uniforms {
    cursor: vec2<f32>,
    mouse_down: u32,
    mouse_press: vec2<f32>,
    mouse_release: vec2<f32>,
    resolution: vec2<f32>,
    time: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

fn image(t: texture_2d<f32>, spl: sampler, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t, spl, vec2(uv.x, 1.0 - uv.y));
}

fn main_image(frag_color: vec4<f32>, frag_coord: vec2<f32>) -> vec4<f32> {
    let uv = frag_coord / u.resolution;
    let color = 0.5 + 0.5 * cos(u.time + uv.xyx + vec3(0.0, 2.0, 4.0));
    return vec4(color, 1.0);
}

@fragment
fn main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let base_color = vec4(0.0, 0.0, 0.0, 1.0);
    let color = main_image(base_color, ((frag_coord.xy - vec2(0.0, u.resolution.y)) * vec2(1.0, -1.0)));
    return vec4(color.rgb, 1.0);
}",
                )),
            });

            let vert_shader = output_surface.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("vertex_shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                    "@vertex
fn main(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32((in_vertex_index << 1u) & 2u));
    let y = f32(i32(in_vertex_index & 2u));
    let out = 2.0 * vec2(x, y) - vec2(1.0);
    return vec4(out, 0.0, 1.0);
}",
                )),
            });

            //let texture_bind_group_layout =
            //    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            //        entries: &[
            //            wgpu::BindGroupLayoutEntry {
            //                binding: 0,
            //                visibility: wgpu::ShaderStages::FRAGMENT,
            //                ty: wgpu::BindingType::Texture {
            //                    multisampled: false,
            //                    view_dimension: wgpu::TextureViewDimension::D2,
            //                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
            //                },
            //                count: None,
            //            },
            //            //wgpu::BindGroupLayoutEntry {
            //            //    binding: 1,
            //            //    visibility: wgpu::ShaderStages::FRAGMENT,
            //            //    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            //            //    count: None,
            //            //},
            //        ],
            //        label: Some("Texture Bind Group Layout"),
            //    });

            let swapchain_capabilities = output_surface.surface.get_capabilities(&output_surface.adapter);
            let swapchain_format = swapchain_capabilities.formats[0];

            let uniform = Uniform::default();

            let uniform_buffer = output_surface.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Uniform Buffer"),
                contents: uniform.as_bytes(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let uniform_bind_group_layout =
                output_surface.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

            let uniform_bind_group = output_surface.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Uniform Bind Group"),
                layout: &uniform_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

            let pipeline_layout = output_surface.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&uniform_bind_group_layout],
                push_constant_ranges: &[],
            });

            let pipeline = output_surface.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vert_shader,
                    entry_point: "main",
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "main",
                    targets: &[Some(swapchain_format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

            let (width, height) = output_surface.output_info.logical_size.expect("illogical size?");
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

            output_surface.surface.configure(&output_surface.device, &surface_config);

            // We don't plan to render much in this example, just clear the surface.
            //let surface_texture = surface
            //    .get_current_texture()
            //    .expect("failed to acquire next swapchain texture");
            //let texture_view = surface_texture
            //    .texture
            //    .create_view(&wgpu::TextureViewDescriptor::default());

            //let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            //    layout: &texture_bind_group_layout,
            //    entries: &[
            //        wgpu::BindGroupEntry {
            //            binding: 0,
            //            resource: wgpu::BindingResource::TextureView(&texture_view),
            //        },
            //        //wgpu::BindGroupEntry {
            //        //    binding: 1,
            //        //    resource: wgpu::BindingResource::Sampler(&sampler),
            //        //},
            //    ],
            //    label: Some("Bind Group"),
            //});

            let mut renderable = Renderable {
                pipeline,
                surface_configuration: surface_config,
                surface_texture: None,
                texture_view: None,
                uniform_bind_group,
                uniform,
                uniform_buffer,
                time_instant: Instant::now(),
            };

            renderable.frame_start(output_surface).expect("first frame kablooey");
            renderable.render(output_surface).expect("first frame kablooey");
            renderable.frame_finish().expect("first frame kablooey");

            output_surface.renderable = Some(renderable);

            //let mut encoder = device.create_command_encoder(&Default::default());
            //{
            //    let mut renderpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            //        label: None,
            //        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            //            view: &texture_view,
            //            resolve_target: None,
            //            ops: wgpu::Operations {
            //                load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
            //                store: true,
            //            },
            //        })],
            //        depth_stencil_attachment: None,
            //    });
            //    renderpass.set_pipeline(&pipeline);
            //    renderpass.set_bind_group(UNIFORM_GROUP_ID, &uniform_bind_group, &[]);
            //    renderpass.draw(0..3, 0..1)
            //}

            //// Submit the command in the queue to execute
            //queue.submit(Some(encoder.finish()));
            //surface_texture.present();
        }
    }

    fn closed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface) {
        todo!()
    }
}

struct Renderable {
    pipeline: RenderPipeline,

    surface_configuration: SurfaceConfiguration,
    surface_texture: Option<SurfaceTexture>,
    texture_view: Option<TextureView>,

    uniform_bind_group: BindGroup,
    uniform: Uniform,
    uniform_buffer: Buffer,
    time_instant: Instant,
}

impl Renderable {
    /// Starts a new frame.
    ///
    /// Needs to be called before [`Self::frame_finish`] and at the begining of each frame.
    ///
    /// # Errors
    ///
    /// - Will return an error if [`Self::frame_finish`] haven't been called at the end of the last frame.
    pub fn frame_start(&mut self, output_surface: &OutputSurface) -> Result<()> {
        if self.surface_texture.is_some() {
            bail!("Non-finished wgpu::SurfaceTexture found.")
        }

        let surface_texture = output_surface
            .surface
            .get_current_texture()
            .expect("couldnt get texture");

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

    fn render(&mut self, output_surface: &OutputSurface) -> Result<()> {
        if self.texture_view.is_none() {
            bail!("No actived wgpu::TextureView found.")
        }

        let view = self.texture_view.as_ref().unwrap();

        let mut encoder =
            output_surface
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

        let (width, height) = output_surface
            .output_info
            .logical_size
            .expect("illogical size?");

        self.uniform.resolution = [width as f32, height as f32];
        self.uniform.time = self.time_instant.elapsed().as_secs_f32();

        output_surface
            .queue
            .write_buffer(&self.uniform_buffer, 0, self.uniform.as_bytes());

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

            render_pass.set_bind_group(UNIFORM_GROUP_ID, &self.uniform_bind_group, &[]);

            //let mut index = 1;
            //for (_, bind_group) in &self.texture_bind_groups {
            //    render_pass.set_bind_group(index, bind_group, &[]);
            //    index += 1;
            //}

            render_pass.draw(0..3, 0..1);
        }

        output_surface.queue.submit(Some(encoder.finish()));

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

impl SeatHandler for Wgpu {
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

delegate_compositor!(Wgpu);
delegate_output!(Wgpu);

delegate_seat!(Wgpu);

delegate_layer!(Wgpu);

delegate_registry!(Wgpu);

impl ProvidesRegistryState for Wgpu {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
