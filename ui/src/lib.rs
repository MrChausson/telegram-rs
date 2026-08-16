//! UI: window (winit), surface (softbuffer) and rendering (tiny-skia).
//! The `ui` crate is testable in isolation (pure rasterization = unit-testable).

pub mod blit;
pub mod bridge;
pub mod chatlist;
pub mod messages;
pub mod renderer;
pub mod state;
pub mod text;
pub mod window;