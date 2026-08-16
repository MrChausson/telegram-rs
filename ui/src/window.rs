//! winit window loop + softbuffer surface + tiny-skia rendering.

use std::num::NonZeroU32;
use std::rc::Rc;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::blit::blit_pixmap;
use crate::bridge::{Request, UiMessage};
use crate::renderer::render;
use crate::state::{Screen, UiState};

const WIDTH: f64 = 980.0;
const HEIGHT: f64 = 720.0;

/// Runs the event loop. `rx` receives network messages; `tx` sends UI
/// requests (open a chat, etc.). With `auto_open_title`, the matching chat is
/// opened automatically once the list loads (for headless visual tests).
pub fn run(
    rx: UnboundedReceiver<UiMessage>,
    tx: UnboundedSender<Request>,
    auto_open_title: Option<String>,
) -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::WaitUntil(
        std::time::Instant::now() + std::time::Duration::from_millis(30),
    ));
    let mut app = App::new(rx, tx, auto_open_title);
    event_loop.run_app(&mut app)
}

struct App {
    window: Option<Rc<Window>>,
    context: Option<softbuffer::Context<Rc<Window>>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    text: crate::text::TextRenderer,
    state: UiState,
    cursor: PhysicalPosition<f64>,
    rx: UnboundedReceiver<UiMessage>,
    tx: UnboundedSender<Request>,
    auto_open: Option<String>,
    /// UI scale factor (logical resolution -> physical pixels).
    /// Overridden by the `TG_UI_SCALE` environment variable if set.
    ui_scale: f32,
    /// Network messages received, to be consumed on the next render pass.
    pending: Vec<UiMessage>,
    /// Pixmap reused between frames (avoids allocating on every render).
    frame: tiny_skia::Pixmap,
}

impl App {
    fn new(rx: UnboundedReceiver<UiMessage>, tx: UnboundedSender<Request>, auto_open: Option<String>) -> Self {
        Self {
            window: None,
            context: None,
            surface: None,
            text: crate::text::TextRenderer::new(),
            state: UiState::new(),
            cursor: PhysicalPosition::new(0.0, 0.0),
            rx,
            tx,
            auto_open,
            ui_scale: 1.0,
            pending: Vec::new(),
            frame: tiny_skia::Pixmap::new(1, 1).expect("pixmap"),
        }
    }

    fn draw(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let (width, height) = {
            let s = window.inner_size();
            (s.width, s.height)
        };
        if width == 0 || height == 0 {
            return;
        }
        let scale = self.ui_scale.max(0.1);

        // Consume network messages (queued by `about_to_wait`).
        for msg in self.pending.drain(..) {
            self.state.on_message(msg);
        }
        // A new message arrived in the open chat: scroll back to the bottom.
        if self.state.take_scroll_bottom() {
            let (lw, lh) = self.logical_size();
            self.state.scroll_messages_to_bottom(&self.text, lw, lh);
        }

        // Test mode: open the target chat as soon as the list is loaded.
        if let Some(target) = &self.auto_open {
            if let Screen::Idle = self.state.screen {
                let id = if target == "*" {
                    self.state.list.rows.first().map(|r| r.id)
                } else {
                    self.state.list.rows.iter().find(|r| &r.title == target).map(|r| r.id)
                };
                if let Some(id) = id {
                    if let Some(req) = self.state.enter_chat(id) {
                        let _ = self.tx.send(req);
                    }
                }
            }
        }

        let Some(surface) = &mut self.surface else {
            return;
        };
        surface
            .resize(
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            )
            .expect("resize surface");

        let mut pixmap = std::mem::replace(&mut self.frame, tiny_skia::Pixmap::new(1, 1).expect("pixmap"));
        if pixmap.width() != width || pixmap.height() != height {
            pixmap = tiny_skia::Pixmap::new(width, height).expect("pixmap");
        }
        if let Err(err) = render(
            &mut pixmap,
            &self.text,
            &self.state.list,
            &self.state.screen,
            &self.state.messages,
            &self.state.status,
            &self.state.input,
            scale,
        ) {
            eprintln!("render failed: {err}");
            return;
        }

        let mut buffer = surface.buffer_mut().expect("buffer");
        if let Err(err) = blit_pixmap(pixmap.as_ref(), &mut buffer, width, height) {
            eprintln!("blit failed: {err}");
            return;
        }
        buffer.present().expect("present");
        self.frame = pixmap;
    }

    /// Logical window size (physical pixels divided by the scale).
    fn logical_size(&self) -> (f32, f32) {
        let scale = self.ui_scale.max(0.1);
        let (w, h) = self
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width as f32, s.height as f32)
            })
            .unwrap_or((WIDTH as f32, HEIGHT as f32));
        (w / scale, h / scale)
    }

    /// Requests a new render (on-demand, not a continuous loop).
    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    /// Periodic poll of the network channel: if new messages arrive, queue
    /// them and trigger a redraw (otherwise stay idle without rendering).
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Refresh the network-poll deadline (otherwise the initial
        // WaitUntil becomes a tight spin loop).
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(30),
        ));
        let mut got = false;
        while let Ok(msg) = self.rx.try_recv() {
            self.pending.push(msg);
            got = true;
        }
        if got {
            self.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("tg")
                        .with_inner_size(LogicalSize::new(WIDTH, HEIGHT)),
                )
                .expect("window creation"),
        );
        window.set_ime_allowed(true);
        let detected = window.scale_factor() as f32;
        // Under X11/XWayland the reported scale factor is often 1.0 even on
        // a HiDPI screen (scaling is applied by the compositor). We fall back
        // to a comfortable default, overridable via `TG_UI_SCALE` (e.g.
        // `TG_UI_SCALE=1.5 app`).
        self.ui_scale = std::env::var("TG_UI_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or_else(|| {
                if detected <= 1.0 {
                    1.6
                } else {
                    detected
                }
            });
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = softbuffer::Surface::new(&context, window.clone())
            .expect("softbuffer surface");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { position, .. } => self.cursor = position,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let scale = self.ui_scale.max(0.1);
                let (w, _) = self.logical_size();
                let req = self.state.click(
                    self.cursor.x as f32 / scale,
                    self.cursor.y as f32 / scale,
                    w,
                );
                if let Some(req) = req {
                    let _ = self.tx.send(req);
                }
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scale = self.ui_scale.max(0.1);
                let (w, h) = self.logical_size();
                // winit convention: positive delta = content moves down
                // (reveals more content above) = scroll toward the bottom.
                // PixelDelta is in physical pixels -> logical conversion.
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 48.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / scale,
                };
                // winit convention: positive delta = content should move
                // down (reveals the start of the conversation). This is the
                // "natural" trackpad direction. `TG_SCROLL_INVERT=1` flips it.
                let invert = std::env::var("TG_SCROLL_INVERT").is_ok_and(|v| v == "1");
                let x = self.cursor.x as f32 / scale;
                self.state.scroll(if invert { dy } else { -dy }, x, w, h, &self.text);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let mut handled = true;
                if event.state == ElementState::Pressed {
                    match &event.logical_key {
                        Key::Character(c) if !c.is_empty() => self.state.push_text(c),
                        Key::Named(NamedKey::Space) => self.state.push_text(" "),
                        Key::Named(NamedKey::Backspace) => self.state.backspace(),
                        Key::Named(NamedKey::Enter) => {
                            if let Some(req) = self.state.enter() {
                                let _ = self.tx.send(req);
                                let (w, h) = self.logical_size();
                                self.state
                                    .scroll_messages_to_bottom(&self.text, w, h);
                            }
                        }
                        _ => handled = false,
                    }
                } else {
                    handled = false;
                }
                if handled {
                    self.request_redraw();
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.state.push_text(&text);
                self.request_redraw();
            }
            WindowEvent::Resized(_) => {
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.draw();
            }
            _ => {}
        }
    }
}