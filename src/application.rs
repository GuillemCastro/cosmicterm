use crate::terminal::Terminal;
use crate::window::WindowState;
use glyphon::Attrs;
use glyphon::Color;
use glyphon::Family;
use glyphon::Resolution;
use glyphon::Shaping;
use glyphon::TextArea;
use glyphon::TextBounds;
use std::sync::Arc;
use std::sync::Mutex;
use wgpu::CommandEncoderDescriptor;
use wgpu::LoadOp;
use wgpu::Operations;
use wgpu::RenderPassColorAttachment;
use wgpu::RenderPassDescriptor;
use wgpu::TextureViewDescriptor;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::ElementState;
use winit::event::StartCause;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::Window;
use winit::window::WindowId;

pub struct Application {
    pub window_state: Option<Arc<Mutex<WindowState>>>,
    terminal: Terminal,
}

impl Application {
    const APP_NAME: &'static str = "cosmicterm";
    pub fn new(terminal: Terminal) -> Self {
        Self {
            window_state: None,
            terminal,
        }
    }
}

impl ApplicationHandler for Application {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        let state = match self.window_state.as_mut() {
            Some(state) => state,
            None => return,
        };
        let state = state.lock().unwrap();
        let window = &state.window;

        match cause {
            StartCause::Poll => {
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window_state.is_some() {
            return;
        }

        let (width, height) = (800, 600);
        let window_attributes = Window::default_attributes()
            .with_inner_size(LogicalSize::new(width as f64, height as f64))
            .with_title(Self::APP_NAME);
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        let window_state = Arc::new(Mutex::new(pollster::block_on(WindowState::new(window))));
        self.window_state = Some(window_state.clone());
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let state = match self.window_state.as_mut() {
            Some(state) => state,
            None => return,
        };

        let mut state = state.lock().unwrap();

        let WindowState {
            window,
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
            ..
        } = &mut *state;

        match event {
            WindowEvent::Resized(size) => {
                // size is physical pixels already
                let phys_w = size.width;
                let phys_h = size.height;

                // reconfigure your surface
                surface_config.width = phys_w;
                surface_config.height = phys_h;
                surface.configure(&device, &surface_config);

                // 1) compute cols/rows in logical space
                let scale = window.scale_factor() as f32;
                let log_w = phys_w as f32 / scale;
                let log_h = phys_h as f32 / scale;

                let font_px = 16.0;
                let cols = (log_w / font_px).floor() as u16;
                let rows = (log_h / font_px).floor() as u16;
                tracing::info!("Resizing terminal to {} cols and {} rows", cols, rows);

                tracing::info!(
                    "phys = {}×{}px, logical = {}×{}px, cols×rows = {}×{}",
                    phys_w,
                    phys_h,
                    log_w,
                    log_h,
                    cols,
                    rows,
                );

                // 2) tell Glyphon the *physical* viewport size
                text_buffer.set_size(font_system, Some(phys_w as f32), Some(phys_h as f32));

                // 3) resize your TTY
                self.terminal.resize(cols, rows).unwrap();

                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                viewport.update(
                    &queue,
                    Resolution {
                        width: surface_config.width,
                        height: surface_config.height,
                    },
                );
                text_buffer.set_text(
                    font_system,
                    &self.terminal.as_text(),
                    &Attrs::new().family(Family::Monospace),
                    Shaping::Advanced,
                );
                text_renderer
                    .prepare(
                        device,
                        queue,
                        font_system,
                        atlas,
                        viewport,
                        [TextArea {
                            buffer: text_buffer,
                            left: 10.0,
                            top: 10.0,
                            scale: window.scale_factor() as f32,
                            bounds: TextBounds::default(),
                            default_color: Color::rgb(255, 255, 255),
                            custom_glyphs: &[],
                        }],
                        swash_cache,
                    )
                    .unwrap();

                let frame = surface.get_current_texture().unwrap();
                let view = frame.texture.create_view(&TextureViewDescriptor::default());
                let mut encoder =
                    device.create_command_encoder(&CommandEncoderDescriptor { label: None });
                {
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(RenderPassColorAttachment {
                            // depth_slice: None,
                            view: &view,
                            resolve_target: None,
                            ops: Operations {
                                load: LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    // Stretch the render‐pass to cover the whole surface!
                    let w = surface_config.width as f32;
                    let h = surface_config.height as f32;
                    pass.set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
                    // (optional) also ensure the scissor covers the full buffer:
                    pass.set_scissor_rect(0, 0, w as u32, h as u32);

                    text_renderer.render(&atlas, &viewport, &mut pass).unwrap();
                }

                queue.submit(Some(encoder.finish()));
                frame.present();

                atlas.trim();
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                device_id: _,
                event,
                is_synthetic,
            } => {
                if is_synthetic || event.state == ElementState::Released {
                    return;
                }
                tracing::info!("Keyboard input: {:?}", event);

                if let Key::Named(NamedKey::Escape) = event.key_without_modifiers() {
                    tracing::info!("Terminal text: {}", self.terminal.as_text());
                    return;
                }

                if let Some(text) = event.text_with_all_modifiers() {
                    tracing::info!("Text input: {:?}", text);
                    self.terminal.write(text.as_bytes());
                } else {
                    let key = event.key_without_modifiers();
                    let data: &[u8] = match key {
                        Key::Named(NamedKey::ArrowUp) => b"\x1B[A",
                        Key::Named(NamedKey::ArrowDown) => b"\x1B[B",
                        Key::Named(NamedKey::ArrowRight) => b"\x1B[C",
                        Key::Named(NamedKey::ArrowLeft) => b"\x1B[D",
                        _ => return,
                    };
                    self.terminal.write(data);
                }
            }
            _ => {}
        }
    }
}
