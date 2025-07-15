use glyphon::Attrs;
use glyphon::Buffer;
use glyphon::Cache;
use glyphon::Family;
use glyphon::FontSystem;
use glyphon::Metrics;
use glyphon::Shaping;
use glyphon::SwashCache;
use glyphon::TextAtlas;
use glyphon::TextRenderer;
use glyphon::Viewport;
use std::sync::Arc;
use wgpu::CompositeAlphaMode;
use wgpu::DeviceDescriptor;
use wgpu::Instance;
use wgpu::InstanceDescriptor;
use wgpu::MultisampleState;
use wgpu::PresentMode;
use wgpu::RequestAdapterOptions;
use wgpu::SurfaceConfiguration;
use wgpu::TextureFormat;
use wgpu::TextureUsages;
use winit::window::Window;

pub struct WindowState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: SurfaceConfiguration,

    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub viewport: glyphon::Viewport,
    pub atlas: glyphon::TextAtlas,
    pub text_renderer: glyphon::TextRenderer,
    pub text_buffer: glyphon::Buffer,

    // Make sure that the winit window is last in the struct so that
    // it is dropped after the wgpu surface is dropped, otherwise the
    // program may crash when closed. This is probably a bug in wgpu.
    pub window: Arc<Window>,
}

impl WindowState {
    pub async fn new(window: Arc<Window>) -> Self {
        let physical_size = window.inner_size();
        let scale_factor = window.scale_factor();

        // Set up surface
        let instance = Instance::new(&InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&RequestAdapterOptions::default())
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor::default())
            .await
            .unwrap();

        let surface = instance
            .create_surface(window.clone())
            .expect("Create surface");
        let swapchain_format = TextureFormat::Bgra8UnormSrgb;
        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: swapchain_format,
            width: physical_size.width,
            height: physical_size.height,
            present_mode: PresentMode::Fifo,
            alpha_mode: CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Set up text renderer
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, swapchain_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 16.0));

        let physical_width = (physical_size.width as f64 * scale_factor) as f32;
        let physical_height = (physical_size.height as f64 * scale_factor) as f32;

        text_buffer.set_size(
            &mut font_system,
            Some(physical_width),
            Some(physical_height),
        );
        text_buffer.set_text(
            &mut font_system,
            "Welcome to cosmicterm :)",
            &Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        text_buffer.shape_until_scroll(&mut font_system, false);

        Self {
            device,
            queue,
            surface,
            surface_config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,
            window,
        }
    }
}
