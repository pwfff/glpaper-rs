use std::error::Error;

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

fn main() -> Result<(), Box<dyn Error>> {
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
                .ok_or_else(|| "output has no info".to_owned())?,
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
        layer.set_anchor(Anchor::BOTTOM);
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

        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None)).expect("couldnt get device");
        OutputSurface {
            layer,
            device,
            surface,
            adapter,
            queue,
        }
    }).collect();

    let mut wgpu = Wgpu {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),

        exit: false,
        width: 256,
        height: 256,
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
    layer: LayerSurface,

    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface,
}

struct Wgpu {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,

    exit: bool,
    width: u32,
    height: u32,

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
        let (new_width, new_height) = configure.new_size;
        if new_width > 0 {
            self.width = new_width;
        }
        if new_height > 0 {
            self.height = new_height;
        }

        for OutputSurface {
            layer,
            adapter,
            surface,
            device,
            queue,
        } in &self.output_surfaces
        {
            if layer.wl_surface().id() != this_layer.wl_surface().id() {
                continue;
            }

            let cap = surface.get_capabilities(&adapter);
            let surface_config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: cap.formats[0],
                view_formats: vec![cap.formats[0]],
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                width: self.width,
                height: self.height,
                // Wayland is inherently a mailbox system.
                present_mode: wgpu::PresentMode::Mailbox,
            };

            surface.configure(device, &surface_config);

            // We don't plan to render much in this example, just clear the surface.
            let surface_texture = surface
                .get_current_texture()
                .expect("failed to acquire next swapchain texture");
            let texture_view = surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _renderpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &texture_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
                            store: true,
                        },
                    })],
                    depth_stencil_attachment: None,
                });
            }

            // Submit the command in the queue to execute
            queue.submit(Some(encoder.finish()));
            surface_texture.present();
        }
    }

    fn closed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface) {
        todo!()
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
