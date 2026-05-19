#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod inference;
mod installer;
mod llm;
mod models;
mod search;
mod update;

use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1180.0, 720.0])
        .with_min_inner_size([760.0, 420.0])
        .with_title("Spellbook");
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Spellbook",
        options,
        Box::new(|cc| {
            // configure fonts so Chinese characters render
            install_cjk_fonts(&cc.egui_ctx);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(app::App::new(cc)))
        }),
    )
}

/// Decode the bundled PNG app icon for the window/Dock.
fn load_app_icon() -> Option<egui::IconData> {
    const PNG: &[u8] = include_bytes!("../assets/icon-256.png");
    let img = image::load_from_memory(PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

/// Bundled NotoEmoji font (OFL license, see assets/OFL.txt).
/// Covers the full Unicode emoji range in monochrome so 📚 ✨ 🧠 ⚙ etc.
/// render reliably without depending on system fonts.
const BUNDLED_EMOJI: &[u8] = include_bytes!("../assets/NotoEmoji-Regular.ttf");

/// Install fonts so CJK characters, decorative emojis, and extended symbols
/// all render. We append to the default family lists, never replace, so:
///   1. Ubuntu-Light still renders Latin
///   2. egui's trimmed NotoEmoji handles common emoji
///   3. our bundled NotoEmoji fills the rest of the emoji range
///   4. system CJK font (PingFang etc) handles Chinese
///   5. system symbols font handles arrows / math
fn install_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Bundled NotoEmoji — always present, no system dependency.
    add_fallback(&mut fonts, "noto-emoji", BUNDLED_EMOJI.to_vec());

    let cjk_candidates = [
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/Library/Fonts/Songti.ttc",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        // Windows
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/simsun.ttc",
    ];
    if let Some(bytes) = first_existing(&cjk_candidates) {
        add_fallback(&mut fonts, "cjk", bytes);
    }

    let symbol_candidates = [
        // macOS — covers many arrows, mathematical, technical symbols
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/System/Library/Fonts/Supplemental/Apple Symbols.ttf",
        // Linux — DejaVu has wide BMP coverage
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        // Windows
        "C:/Windows/Fonts/seguisym.ttf",
    ];
    if let Some(bytes) = first_existing(&symbol_candidates) {
        add_fallback(&mut fonts, "symbols", bytes);
    }

    ctx.set_fonts(fonts);
}

fn first_existing(paths: &[&str]) -> Option<Vec<u8>> {
    for p in paths {
        if let Ok(bytes) = std::fs::read(p) {
            return Some(bytes);
        }
    }
    None
}

fn add_fallback(fonts: &mut egui::FontDefinitions, name: &str, bytes: Vec<u8>) {
    fonts
        .font_data
        .insert(name.to_owned(), egui::FontData::from_owned(bytes));
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push(name.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push(name.to_owned());
}
