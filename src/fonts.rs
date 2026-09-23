use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    if let Some(path) = mono_file() {
        if let Ok(data) = std::fs::read(&path) {
            fonts
                .font_data
                .insert("omarchy-mono".into(), Arc::new(FontData::from_owned(data)));
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                fam.insert(0, "omarchy-mono".into());
            }
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                fam.insert(0, "omarchy-mono".into());
            }
        }
    }

    if let Some(data) = load_first(&[
        "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/TTF/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    ]) {
        fonts
            .font_data
            .insert("serif".into(), Arc::new(FontData::from_owned(data)));
        fonts
            .families
            .insert(FontFamily::Name("serif".into()), vec!["serif".into()]);
    }

    // Noto Color Emoji is a CBDT bitmap: egui extracts no outlines from it,
    // and the "emoji" family swallowed glyphs without drawing anything.
    // Shelf icons go through `emoji::Atlas`.

    ctx.set_fonts(fonts);
}

/// System mono file — the same `monospace` alias as the Omarchy terminal.
pub fn mono_file() -> Option<PathBuf> {
    let out = Command::new("fc-match")
        .args(["monospace", "-f", "%{file}\n"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        None
    } else {
        Some(PathBuf::from(p))
    }
}

fn load_first(paths: &[&str]) -> Option<Vec<u8>> {
    for p in paths {
        if let Ok(b) = std::fs::read(p) {
            return Some(b);
        }
    }
    None
}
