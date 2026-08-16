//! winit window loop + softbuffer surface + tiny-skia rendering.

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::rc::Rc;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::blit::blit_pixmap;
use crate::bridge::{Request, UiMessage};
use crate::image::PhotoCache;
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
    /// Decoded photo thumbnails (LRU, bounded memory).
    photos: PhotoCache,
    /// (chat, message) pairs already asked to be downloaded.
    requested_photos: HashSet<(i64, i32)>,
    /// Chat ids already asked for a profile photo.
    requested_avatars: HashSet<i64>,
    /// Current keyboard modifiers (kept for Ctrl shortcuts, including in IME).
    mods: ModifiersState,
    /// True while the left mouse button is held.
    mouse_down: bool,
    /// True once a press has turned into a text-selection drag.
    dragging_selection: bool,
    /// Logical position of the last left-button press.
    press: (f32, f32),
    /// System clipboard (created lazily; `None` when unavailable).
    clipboard: Option<arboard::Clipboard>,
    /// Debug FPS meter: `fps.last_frame` = time of the previous draw,
    /// `fps.ema_ms` = exponential average frame time (for the overlay/log).
    fps: FpsMeter,
}

/// Rolling frame-time meter (1-second window) used when `TG_FPS=1`.
struct FpsMeter {
    last_frame: std::time::Instant,
    ema_ms: f32,
}

impl FpsMeter {
    fn new() -> Self {
        Self {
            last_frame: std::time::Instant::now(),
            ema_ms: 0.0,
        }
    }

    /// Reports the ms taken by the frame that just ended, maintaining a
    /// running average (and returning the instantaneous ms).
    fn frame(&mut self, elapsed: f32) {
        // Exponential average half-life ≈ 30 frames.
        self.ema_ms = self.ema_ms * 0.95 + elapsed * 0.05;
    }
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
            photos: PhotoCache::new(),
            requested_photos: HashSet::new(),
            requested_avatars: HashSet::new(),
            mods: ModifiersState::default(),
            mouse_down: false,
            dragging_selection: false,
            press: (0.0, 0.0),
            clipboard: None,
            fps: FpsMeter::new(),
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
        let fps_debug = std::env::var("TG_FPS").is_ok_and(|v| v == "1");
        let frame_start = std::time::Instant::now();

        // Consume network messages (queued by `about_to_wait`).
        for msg in self.pending.drain(..) {
            self.state.on_message(msg);
        }
        // A new message arrived in the open chat: scroll back to the bottom.
        if self.state.take_scroll_bottom() {
            let (lw, lh) = self.logical_size();
            self.state.scroll_messages_to_bottom(&self.text, lw, lh);
        }

        // Request profile photos for chats currently visible in the list
        // (requesting every row at once would flood the network queue and
        // delay the open chat request behind it).
        let mut avatar_requests = Vec::new();
        let scroll = self.state.list.scroll;
        let row_h = self.state.list.row_height;
        let first = (scroll / row_h).floor().max(0.0) as usize;
        let visible = (self.logical_size().1 / row_h).ceil() as usize + 8;
        for i in first..first + visible {
            let Some(row) = self.state.list.rows.get(i) else {
                break;
            };
            if row.avatar_path.is_none() && !self.requested_avatars.contains(&row.id) {
                self.requested_avatars.insert(row.id);
                avatar_requests.push(row.id);
            }
        }
        for id in avatar_requests {
            let _ = self.tx.send(Request::DownloadAvatar { chat_id: id });
        }

        if self.state.authenticated {
            // Request thumbnails for messages that have a photo but no file yet.
            if let Screen::Chat { id, .. } = self.state.screen {
                for row in &self.state.messages.rows {
                    if row.photo.is_some() && row.photo_path.is_none()
                        && !self.requested_photos.contains(&(id, row.id))
                    {
                        self.requested_photos.insert((id, row.id));
                        let _ = self.tx.send(Request::DownloadPhoto {
                            chat_id: id,
                            msg_id: row.id,
                        });
                    }
                }
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
                            self.requested_photos.clear();
                            let _ = self.tx.send(req);
                        }
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
        let login = if self.state.authenticated {
            None
        } else {
            Some(crate::renderer::LoginView {
                step: &self.state.login_step,
                input: &self.state.login_input,
                status: &self.state.status,
                error: self.state.login_error,
            })
        };
        if let Err(err) = render(
            &mut pixmap,
            &self.text,
            &self.state.list,
            &self.state.screen,
            &self.state.messages,
            &self.state.status,
            &self.state.input,
            self.state.viewer.as_deref(),
            &self.photos,
            login,
            self.state.selection(),
            scale,
        ) {
            eprintln!("render failed: {err}");
            return;
        }

        let mut buffer = surface.buffer_mut().expect("buffer");
        let blit_start = std::time::Instant::now();
        if let Err(err) = blit_pixmap(pixmap.as_ref(), &mut buffer, width, height) {
            eprintln!("blit failed: {err}");
            return;
        }
        buffer.present().expect("present");
        self.frame = pixmap;
        let blit_ms = blit_start.elapsed().as_secs_f32() * 1000.0;

        if fps_debug {
            let dt = frame_start.elapsed().as_secs_f32();
            let frame_ms = dt * 1000.0;
            self.fps.frame(frame_ms);
            let fps = 1000.0 / self.fps.ema_ms.max(0.001);
            eprintln!(
                "[fps] {fps:5.1} fps  {frame_ms:6.2} ms/frame (render+blit)  blit {blit_ms:6.2} ms  {width}x{height} (scale {scale:.2})"
            );
        }
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
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                // Extend the text selection while dragging.
                if self.mouse_down {
                    let scale = self.ui_scale.max(0.1);
                    let (w, _h) = self.logical_size();
                    let lx = self.cursor.x as f32 / scale;
                    let ly = self.cursor.y as f32 / scale;
                    if !self.dragging_selection {
                        // Turn the press into a selection drag only after a
                        // small movement, so a plain click keeps its behavior.
                        let dx = lx - self.press.0;
                        let dy = ly - self.press.1;
                        if dx * dx + dy * dy > 9.0
                            && self.state.begin_selection(&self.text, self.press.0, self.press.1, w)
                        {
                            self.dragging_selection = true;
                        }
                    }
                    if self.dragging_selection {
                        self.state.update_selection(&self.text, lx, ly, w);
                        self.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    let scale = self.ui_scale.max(0.1);
                    let lx = self.cursor.x as f32 / scale;
                    let ly = self.cursor.y as f32 / scale;
                    self.mouse_down = true;
                    self.press = (lx, ly);
                    self.dragging_selection = false;
                }
                ElementState::Released => {
                    if !self.mouse_down {
                        return;
                    }
                    self.mouse_down = false;
                    if self.dragging_selection {
                        self.dragging_selection = false;
                        self.request_redraw();
                        return;
                    }
                    // Plain click: selection is cleared, existing actions run.
                    let scale = self.ui_scale.max(0.1);
                    let (w, h) = self.logical_size();
                    let lx = self.cursor.x as f32 / scale;
                    let ly = self.cursor.y as f32 / scale;
                    let req = self.state.click(lx, ly, w, h);
                    if let Some(req) = req {
                        if matches!(req, Request::OpenChat { .. }) {
                            // Re-entering a chat must re-request thumbnails
                            // whose row was reset with `photo_path: None`.
                            self.requested_photos.clear();
                        }
                        let _ = self.tx.send(req);
                    }
                    if self.state.viewer.is_some() {
                        self.state.close_viewer();
                    } else if let Some(path) = self.state.photo_at(lx, ly, w) {
                        self.state.open_viewer(path);
                    }
                    self.state.clear_selection();
                    self.request_redraw();
                }
            },
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
                    let ctrl = self.mods.control_key() || self.mods.super_key();
                    match &event.logical_key {
                        Key::Character(c) if !c.is_empty() && ctrl => {
                            match c.to_ascii_lowercase().as_str() {
                                "v" => self.paste_clipboard(),
                                "c" => self.copy_to_clipboard(),
                                _ => handled = false,
                            }
                        }
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
                        Key::Named(NamedKey::Escape) => self.state.clear_selection(),
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
                // Ctrl shortcuts (e.g. Ctrl+V) arrive here as a committed
                // character on some platforms; never type them.
                if !self.mods.control_key() {
                    self.state.push_text(&text);
                    self.request_redraw();
                }
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

impl App {
    /// Copies the current selection (else the hovered message, else the input
    /// field) to the system clipboard.
    fn copy_to_clipboard(&mut self) {
        let scale = self.ui_scale.max(0.1);
        let (w, _h) = self.logical_size();
        let lx = self.cursor.x as f32 / scale;
        let ly = self.cursor.y as f32 / scale;
        let Some(text) = self.state.copy_source(&self.text, lx, ly, w) else {
            return;
        };
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        if let Some(clip) = self.clipboard.as_mut() {
            let _ = clip.set_text(text);
        }
    }

    /// Pastes the system clipboard into the active text field.
    fn paste_clipboard(&mut self) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        let Some(clip) = self.clipboard.as_mut() else {
            return;
        };
        if let Ok(text) = clip.get_text() {
            self.state.push_text(&text);
        }
    }
}