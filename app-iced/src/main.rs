//! Thin binary wrapper around the `app_iced` library: the whole UI lives in
//! `lib.rs` so the `benches/` harness can drive the same view code headlessly.

fn main() -> iced::Result {
    app_iced::run()
}
