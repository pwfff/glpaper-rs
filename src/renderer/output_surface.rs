use std::borrow::Cow;
use std::fs::File;
use std::io::Read;
use std::mem::size_of;
use std::path::Path;
use std::time::{Duration, Instant};

use super::download;
use anyhow::Result;
use image::ImageBuffer;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use sctk::shell::{wlr_layer::LayerSurface, WaylandSurface};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};
use wgpu::util::DeviceExt;
use wgpu::{Maintain, MaintainBase, SubmissionIndex, SurfaceTexture};

// TODO: add these
// All unsupported uniforms. Attempting to use any of these in a shader will result in an error.
pub static UNSUPPORTED_UNIFORMS: [&str; 5] = [
    "iTimeDelta",
    "iChannelTime",
    "iChannelResolution",
    "iDate",
    "iSampleRate",
    // broken because https://github.com/gfx-rs/naga/issues/1012
    //"iChannel0",
    //"iChannel1",
    //"iChannel2",
    //"iChannel3",
];

pub struct OutputSurface {
    start_time: Instant,
    submitted_frame: Option<(SurfaceTexture, SubmissionIndex)>,

    exp: f32,
    globals: IGlobals,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface,
    pipe: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    swapchain_format: wgpu::TextureFormat,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    num_indices: u32,
}

trait Binding {
    fn layout(&self) -> wgpu::BindingType;
    fn binding(&self) -> wgpu::BindingResource;
}

pub struct BufferBinding<H> {
    pub host: H,
    //serialise: Box<dyn for<'a> Fn(&'a H) -> &'a [u8]>,
    serialise: Box<dyn Fn(&H) -> Vec<u8>>,
    device: wgpu::Buffer,
    layout: wgpu::BindingType,
    bind: Box<dyn for<'a> Fn(&'a wgpu::Buffer) -> wgpu::BufferBinding<'a>>,
}

impl<H> Drop for BufferBinding<H> {
    fn drop(&mut self) {
        self.device.destroy();
    }
}

impl<H> Binding for BufferBinding<H> {
    fn layout(&self) -> wgpu::BindingType {
        self.layout
    }
    fn binding(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::Buffer((self.bind)(&self.device))
    }
}

impl<H> BufferBinding<H> {
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.device
    }
    fn stage(&self, queue: &wgpu::Queue) {
        let data = (self.serialise)(&self.host);
        if !data.is_empty() {
            queue.write_buffer(&self.device, 0, &data)
        } else {
            println!("no data to stage")
        }
    }
}

pub struct TextureBinding {
    device: wgpu::Texture,
    view: wgpu::TextureView,
    layout: wgpu::BindingType,
}

impl Drop for TextureBinding {
    fn drop(&mut self) {
        self.device.destroy();
    }
}

impl Binding for TextureBinding {
    fn layout(&self) -> wgpu::BindingType {
        self.layout
    }
    fn binding(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::TextureView(&self.view)
    }
}

impl TextureBinding {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.device
    }
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
    pub fn set_texture(&mut self, texture: wgpu::Texture) {
        self.device = texture;
        self.view = self.device.create_view(&Default::default());
    }
}

struct SamplerBinding {
    layout: wgpu::BindingType,
    bind: wgpu::Sampler,
}

impl Binding for SamplerBinding {
    fn layout(&self) -> wgpu::BindingType {
        self.layout
    }
    fn binding(&self) -> wgpu::BindingResource {
        wgpu::BindingResource::Sampler(&self.bind)
    }
}

struct IGlobals {
    // Uniforms.
    i_global_time: BufferBinding<f32>,
    i_time: BufferBinding<f32>,
    i_resolution: BufferBinding<[f32; 3]>,
    i_mouse: BufferBinding<[f32; 4]>,
    i_frame: BufferBinding<i32>,
    channels: [Texture; 4],
}

impl IGlobals {
    pub fn new(
        av: &ArgValues,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Self {
        let uniform_buffer = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };

        let channels = [
            load_texture(DEFAULT_TEXTURE0_BUF, &av.texture0path, device, queue).unwrap(),
            load_texture(DEFAULT_TEXTURE1_BUF, &av.texture1path, device, queue).unwrap(),
            load_texture(DEFAULT_TEXTURE2_BUF, &av.texture2path, device, queue).unwrap(),
            load_texture(DEFAULT_TEXTURE3_BUF, &av.texture3path, device, queue).unwrap(),
        ];

        //let storage_buffer = wgpu::BindingType::Buffer {
        //    ty: wgpu::BufferBindingType::Storage { read_only: false },
        //    has_dynamic_offset: false,
        //    min_binding_size: None,
        //};
        //let pass_format = "rgba32float";
        //if pass_f32 {
        //    "rgba32float"
        //} else {
        //    "rgba16float"
        //};
        //let blank = wgpu::TextureDescriptor {
        //    size: wgpu::Extent3d {
        //        width: 1,
        //        height: 1,
        //        depth_or_array_layers: 1,
        //    },
        //    mip_level_count: 1,
        //    sample_count: 1,
        //    dimension: wgpu::TextureDimension::D2,
        //    format: wgpu::TextureFormat::Rgba8UnormSrgb,
        //    usage: wgpu::TextureUsages::TEXTURE_BINDING,
        //    label: None,
        //    view_formats: &[],
        //};
        //let channel_layout = wgpu::BindingType::Texture {
        //    multisampled: false,
        //    sample_type: wgpu::TextureSampleType::Float { filterable: true },
        //    view_dimension: wgpu::TextureViewDimension::D2,
        //};
        //let repeat = wgpu::SamplerDescriptor {
        //    address_mode_u: wgpu::AddressMode::Repeat,
        //    address_mode_v: wgpu::AddressMode::Repeat,
        //    address_mode_w: wgpu::AddressMode::Repeat,
        //    ..Default::default()
        //};
        IGlobals {
            i_global_time: BufferBinding {
                host: 0.,
                serialise: Box::new(|h| bytemuck::bytes_of(h).to_vec()),
                device: device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: size_of::<f32>() as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                }),
                layout: uniform_buffer,
                bind: Box::new(wgpu::Buffer::as_entire_buffer_binding),
            },
            i_time: BufferBinding {
                host: 0.,
                serialise: Box::new(|h| bytemuck::bytes_of(h).to_vec()),
                device: device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: size_of::<f32>() as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                }),
                layout: uniform_buffer,
                bind: Box::new(wgpu::Buffer::as_entire_buffer_binding),
            },
            i_resolution: BufferBinding {
                host: [width as f32, height as f32, 0.],
                serialise: Box::new(|h| bytemuck::bytes_of(h).to_vec()),
                device: device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: size_of::<[f32; 3]>() as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                }),
                layout: uniform_buffer,
                bind: Box::new(wgpu::Buffer::as_entire_buffer_binding),
            },
            i_mouse: BufferBinding {
                host: [0.; 4],
                serialise: Box::new(|h| bytemuck::bytes_of(h).to_vec()),
                device: device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: size_of::<[f32; 4]>() as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                }),
                layout: uniform_buffer,
                bind: Box::new(wgpu::Buffer::as_entire_buffer_binding),
            },
            i_frame: BufferBinding {
                host: 0,
                serialise: Box::new(|h| bytemuck::bytes_of(h).to_vec()),
                device: device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: size_of::<i32>() as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: false,
                }),
                layout: uniform_buffer,
                bind: Box::new(wgpu::Buffer::as_entire_buffer_binding),
            },
            channels,
        }
    }

    fn to_vec(&self) -> Vec<&dyn Binding> {
        vec![
            &self.i_global_time,
            &self.i_time,
            &self.i_resolution,
            &self.i_mouse,
            &self.i_frame,
            &self.channels[0].texture,
            &self.channels[0].sampler,
            &self.channels[1].texture,
            &self.channels[1].sampler,
            &self.channels[2].texture,
            &self.channels[2].sampler,
            &self.channels[3].texture,
            &self.channels[3].sampler,
        ]
    }

    fn stage(&self, queue: &wgpu::Queue) {
        self.i_global_time.stage(queue);
        self.i_time.stage(queue);
        self.i_resolution.stage(queue);
        self.i_mouse.stage(queue);
        self.i_frame.stage(queue);
    }
}

const SCREEN: [[f32; 2]; 4] = [
    [1.0, 1.0],   // Top right.
    [-1.0, 1.0],  // Top left.
    [-1.0, -1.0], // Bottom left.
    [1.0, -1.0],  // Bottom right.
];

const SCREEN_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

const CLEAR_COLOR: [f32; 4] = [1.0; 4];

// Default shaders.
pub static DEFAULT_VERT_SRC_BUF: &str = include_str!("../../shaders/default.vert");
pub static DEFAULT_FRAG_SRC_STR: &str = include_str!("../../examples/seascape.frag");

// Default textures.
pub static DEFAULT_TEXTURE0_BUF: &[u8] = include_bytes!("../../textures/01-brickwall.jpg");
pub static DEFAULT_TEXTURE1_BUF: &[u8] = include_bytes!("../../textures/02-landscape.jpg");
pub static DEFAULT_TEXTURE2_BUF: &[u8] = include_bytes!("../../textures/03-whitenoise.jpg");
pub static DEFAULT_TEXTURE3_BUF: &[u8] = include_bytes!("../../textures/04-woodgrain.jpg");

// Example shaders.
pub static EXAMPLE_SEASCAPE_STR: &str = include_str!("../../examples/seascape.frag");
pub static EXAMPLE_ELEMENTAL_RING_STR: &str = include_str!("../../examples/elemental-ring.frag");

// Fragment shader prefix.
const PREFIX: &str = "
#version 440 core

layout(binding=0) uniform float      iGlobalTime;
layout(binding=1) uniform float      iTime;
layout(binding=2) uniform vec3       iResolution;
layout(binding=3) uniform vec4       iMouse;
layout(binding=4) uniform int        iFrame;

layout(binding=5) uniform texture2D  iChannel0_tex;
layout(binding=6) uniform sampler    iChannel0_sam;
layout(binding=7) uniform texture2D  iChannel1_tex;
layout(binding=8) uniform sampler    iChannel1_sam;
layout(binding=9) uniform texture2D  iChannel2_tex;
layout(binding=10) uniform sampler   iChannel2_sam;
layout(binding=11) uniform texture2D iChannel3_tex;
layout(binding=12) uniform sampler   iChannel3_sam;

layout(location=0) in vec2 fragCoord;
layout(location=0) out vec4 fragColor;

#define iChannel0  sampler2D(iChannel0_tex, iChannel0_sam)
#define iChannel1  sampler2D(iChannel1_tex, iChannel1_sam)
#define iChannel2  sampler2D(iChannel2_tex, iChannel2_sam)
#define iChannel3  sampler2D(iChannel3_tex, iChannel3_sam)
";

// Fragment shader suffix.
const SUFFIX: &str = "
void main() {
    fragColor = vec4(1.0, 1.0, 0.0, 0.0);
    mainImage(fragColor, fragCoord);
}
";

#[derive(Default)]
pub struct ArgValues {
    // Path to the shader. None if using default fragment shader.
    pub shaderpath: Option<String>,

    // Path to the n-th texture. None if using default textures.
    pub texture0path: Option<String>,
    pub texture1path: Option<String>,
    pub texture2path: Option<String>,
    pub texture3path: Option<String>,

    // Wrap mode for the n-th texture. Defaults to "clamp" if unspecified.
    pub wrap0: wgpu::AddressMode,
    pub wrap1: wgpu::AddressMode,
    pub wrap2: wgpu::AddressMode,
    pub wrap3: wgpu::AddressMode,

    // Filter method for the n-th texture. Defaults to "mipmap" if unspecified.
    pub filter0: wgpu::FilterMode,
    pub filter1: wgpu::FilterMode,
    pub filter2: wgpu::FilterMode,
    pub filter3: wgpu::FilterMode,

    // Max value for anisotropic filtering. Defaults to 1 if unspecified. Only needed for
    // "anisotropic" filter method.
    pub anisotropic_max: u8,

    // Some(name) if running an example.
    pub examplename: Option<String>,

    // Shadertoy id if downloading a shader.
    pub getid: Option<String>,
}

pub fn format_shader_src(src: &str) -> String {
    format!("{}\n{}\n{}", PREFIX, src, SUFFIX).into()
}

pub fn load_fragment_shader(av: &ArgValues) -> Result<String, String> {
    let frag_src_str = if let Some(ref example) = av.examplename.as_ref() {
        match example.as_ref() {
            "seascape" => EXAMPLE_SEASCAPE_STR.to_string(),
            "elemental-ring" => EXAMPLE_ELEMENTAL_RING_STR.to_string(),
            _ => return Err(format!("no such example {}", example)),
        }
    } else {
        // Read fragment shader from file into String buffer.
        match av.shaderpath {
            Some(ref shaderpath) => {
                let mut frag_src_str = String::new();

                File::open(&Path::new(&shaderpath))
                    .or_else(|err| Err(format!("could not open {}: {:?}", shaderpath, err)))?
                    .read_to_string(&mut frag_src_str)
                    .or_else(|err| Err(format!("could not read {}: {:?}", shaderpath, err)))?;

                frag_src_str
            }
            None => String::from(DEFAULT_FRAG_SRC_STR),
        }
    };

    let unsupported_uniforms: Vec<String> = UNSUPPORTED_UNIFORMS
        .iter()
        .map(|s| s.to_string())
        .filter(|uu| frag_src_str.contains(uu))
        .collect();

    if unsupported_uniforms.is_empty() {
        Ok(format_shader_src(&frag_src_str))
    } else {
        Err(format!("unsupported uniforms: {:?}", unsupported_uniforms))
    }
}

pub fn load_vertex_shader() -> Cow<'static, str> {
    DEFAULT_VERT_SRC_BUF.into()
}

pub struct Texture {
    texture: TextureBinding,
    sampler: SamplerBinding,
}

impl Texture {
    pub fn from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &[u8],
        label: Option<&str>,
    ) -> Result<Self> {
        let img = image::load_from_memory(img)?;
        Self::from_image(device, queue, &img.to_rgba8(), label)
    }

    pub fn from_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
        label: Option<&str>,
    ) -> Result<Self> {
        let dimensions = (img.width(), img.height());

        let size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                aspect: wgpu::TextureAspect::All,
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            &img,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            texture: TextureBinding {
                view,
                device: texture,
                layout: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
            },
            sampler: SamplerBinding {
                bind: sampler,
                layout: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            },
        })
    }
}

pub fn load_texture(
    default_buf: &[u8],
    texpath: &Option<String>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<Texture, String> {
    let img = if let Some(path) = texpath {
        image::open(&path.clone())
            .map_err(|e| format!("{:?}", e))?
            .flipv()
            .to_rgba8()
    } else {
        image::load_from_memory(default_buf)
            .map_err(|e| format!("{:?}", e))?
            .flipv()
            .to_rgba8()
    };

    println!("tex from image");

    let t = Texture::from_image(device, queue, &img, None).map_err(|e| format!("{:?}", e));

    println!("hmmm");

    t

    //let t = device.create_texture(&wgpu::TextureDescriptor {
    //    label: None,
    //    size: wgpu::Extent3d {
    //        width: img.width(),
    //        height: img.height(),
    //        depth_or_array_layers: 1,
    //    },
    //    mip_level_count: 1,
    //    sample_count: 1,
    //    dimension: wgpu::TextureDimension::D2,
    //    format: wgpu::TextureFormat::Rgba8Uint,
    //    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    //    view_formats: &[],
    //});

    //queue.write_texture(
    //    t.texture().as_image_copy(),
    //    img.as_raw(),
    //    wgpu::ImageDataLayout {
    //        offset: 0,
    //        bytes_per_row: Some(4 * img.width()),
    //        rows_per_image: Some(img.height()),
    //    },
    //    wgpu::Extent3d {
    //        width: img.width(),
    //        height: img.height(),
    //        depth_or_array_layers: 1,
    //    },
    //);
}

impl OutputSurface {
    pub(crate) async fn new(
        conn: Connection,
        layer: &LayerSurface,
        width: u32,
        height: u32,
        shader_id: Option<String>,
    ) -> Result<Self, String> {
        let mut av = ArgValues {
            getid: shader_id,
            ..Default::default()
        };
        let vert_src_buf = load_vertex_shader();
        let frag_src_buf = if av.getid.is_some() {
            if av.getid.clone().unwrap().contains(".") {
                av.shaderpath = av.getid.clone();
                load_fragment_shader(&av)?
            } else {
                let (_, shadercode) = download::download(&mut av)
                    .await
                    .map_err(|e| format!("{}", e))?;
                format_shader_src(&shadercode)
            }
        } else {
            load_fragment_shader(&av)?
        };

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

        //
        //
        //

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Glsl {
                shader: vert_src_buf,
                stage: naga::ShaderStage::Vertex,
                defines: Default::default(),
            },
        });

        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Glsl {
                shader: frag_src_buf.into(),
                stage: naga::ShaderStage::Fragment,
                defines: Default::default(),
            },
        });

        let globals = IGlobals::new(&av, &device, &queue, width, height);
        let globals_vec = globals.to_vec();

        let needs_mipmap = |mode: wgpu::FilterMode| {
            mode != wgpu::FilterMode::Nearest && mode != wgpu::FilterMode::Linear
        };

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &globals_vec
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupLayoutEntry {
                    binding: i as u32,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: b.layout(),
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &globals_vec
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.binding(),
                })
                .collect::<Vec<_>>(),
        });

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&SCREEN),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&SCREEN_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let num_indices = SCREEN_INDICES.len() as u32;

        println!("creatin pipeline");

        //let mut encoder = device.create_command_encoder(&Default::default());
        let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &vert,
                entry_point: "main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        // verbose, could be written in a more concise way with vertex_attr_array! macro
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag,
                entry_point: "main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: swapchain_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        println!("created pipeline");

        //let (vertex_buffer, slice) =
        //    factory.create_vertex_buffer_with_slice(&SCREEN, &SCREEN_INDICES[..]);

        //// Load textures.
        //let texture0 = loader::load_texture(&TextureId::Zero, &av.texture0path, &mut factory)?;
        //let texture1 = loader::load_texture(&TextureId::One, &av.texture1path, &mut factory)?;
        //let texture2 = loader::load_texture(&TextureId::Two, &av.texture2path, &mut factory)?;
        //let texture3 = loader::load_texture(&TextureId::Three, &av.texture3path, &mut factory)?;

        //let needs_mipmap =
        //    |mode: FilterMethod| mode != FilterMethod::Scale && mode != FilterMethod::Bilinear;

        //// Generate mipmaps if needed.
        //if needs_mipmap(av.filter0) {
        //    encoder.generate_mipmap(&texture0)
        //};
        //if needs_mipmap(av.filter1) {
        //    encoder.generate_mipmap(&texture1)
        //};
        //if needs_mipmap(av.filter2) {
        //    encoder.generate_mipmap(&texture2)
        //};
        //if needs_mipmap(av.filter3) {
        //    encoder.generate_mipmap(&texture3)
        //};

        //let mut data = pipe::Data {
        //    vbuf: vertex_buffer,

        //    i_global_time: 0.0,
        //    i_time: 0.0,
        //    i_resolution: [width, height, width / height],
        //    i_mouse: [0.0; 4],
        //    i_frame: -1,

        //    i_channel0: (
        //        texture0,
        //        factory.create_sampler(texture::SamplerInfo::new(av.filter0, av.wrap0)),
        //    ),
        //    i_channel1: (
        //        texture1,
        //        factory.create_sampler(texture::SamplerInfo::new(av.filter1, av.wrap1)),
        //    ),
        //    i_channel2: (
        //        texture2,
        //        factory.create_sampler(texture::SamplerInfo::new(av.filter2, av.wrap2)),
        //    ),
        //    i_channel3: (
        //        texture3,
        //        factory.create_sampler(texture::SamplerInfo::new(av.filter3, av.wrap3)),
        //    ),
        //};

        // Generate mipmaps if needed.
        //if needs_mipmap(av.filter0) {
        //    encoder.generate_mipmap(&texture0)
        //};
        //if needs_mipmap(av.filter1) {
        //    encoder.generate_mipmap(&texture1)
        //};
        //if needs_mipmap(av.filter2) {
        //    encoder.generate_mipmap(&texture2)
        //};
        //if needs_mipmap(av.filter3) {
        //    encoder.generate_mipmap(&texture3)
        //};

        println!("well it compiled?");

        Ok(Self {
            device,
            queue,
            pipe,
            bind_group,
            surface,
            swapchain_format,
            vbuf,
            ibuf,
            num_indices,
            globals,
            start_time: Instant::now(),
            submitted_frame: None,
            exp: 0.9,
        })
    }

    //fn custom_floats_vec(fs: Vec<Uniform>) -> (Vec<String>, Vec<f32>) {
    //    fs.iter().fold((vec![], vec![]), |(mut ks, mut vs), u| {
    //        ks.push(u.name.clone());
    //        vs.push(u.value);
    //        (ks, vs)
    //    })
    //}

    pub fn set_fft(&mut self, med_fv: f32, max_fv: f32) {
        self.globals.i_mouse.host[0] = max_fv.max(self.globals.i_mouse.host[0]);
        self.globals.i_mouse.host[1] = med_fv.max(self.globals.i_mouse.host[1]);
        self.start_time -= Duration::from_secs_f32(med_fv / 10.);
        //let mut fs = self.original_uniforms.to_vec();
        //self.exp = med_fv.max(0.1).max(self.exp) * 0.75;
        //for u in fs.iter_mut() {
        //    if u.name == "Exposure" {
        //        u.value = self.exp;
        //    }
        //    if u.name == "Samples" {
        //        u.value = 0.2;
        //    }
        //}
        //let (names, values) = Self::custom_floats_vec(fs);
        //self.toy.set_custom_floats(names, values)
    }

    pub fn load_shader(&mut self) -> Result<()> {
        Ok(())
    }

    pub fn reset(&mut self) -> Result<()> {
        //self.toy.reset();
        Ok(())
    }

    pub fn draw(&mut self) -> Result<()> {
        // TODO: is it ok if we only poll when actually rendering?
        self.device.poll(Maintain::Poll);

        if self.submitted_frame.is_some() {
            return Ok(());
        }
        let time = self.start_time.elapsed().as_secs_f32();
        self.globals.i_time.host = time;
        self.globals.i_global_time.host = time;
        let frame = self.surface.get_current_texture()?;
        let view = &frame.texture.create_view(&Default::default());
        self.globals.stage(&self.queue);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });
            render.set_pipeline(&self.pipe);
            render.set_bind_group(0, &self.bind_group, &[]);
            render.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint16);
            render.set_vertex_buffer(0, self.vbuf.slice(..));
            //render.draw(0..1, 0..1);
            render.draw_indexed(0..self.num_indices, 0, 0..1);
        }
        let buf = Some(encoder.finish());
        let i = self.queue.submit(buf);
        //let (_, i) = self.toy.render_to_surface(&frame);
        self.submitted_frame = Some((frame, i));

        self.device.poll(MaintainBase::Poll);

        self.globals.i_mouse.host[0] *= 0.8;
        self.globals.i_mouse.host[1] *= 0.8;

        Ok(())
    }

    pub fn wait(&mut self) -> Result<()> {
        if let Some((_, i)) = &self.submitted_frame {
            self.device
                .poll(MaintainBase::WaitForSubmissionIndex(i.clone()));
        }

        Ok(())
    }

    pub fn render(&mut self, layer: &WlSurface) -> Result<()> {
        if let Some((frame, i)) = self.submitted_frame.take() {
            self.device.poll(Maintain::WaitForSubmissionIndex(i));
            frame.present();
        }
        layer.commit();

        Ok(())
    }
}
