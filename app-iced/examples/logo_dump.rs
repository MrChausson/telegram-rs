//! Dumps the code-rendered app logo to a PNG (visual QA helper).
fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "logo.png".into());
    let px: u32 = 128;
    let pixmap = app_iced::icons::render_logo_rgba(px);
    std::fs::write(&out, pixmap.encode_png().unwrap()).unwrap();
    println!("wrote {out}");
}
