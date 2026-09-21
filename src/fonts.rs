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

    // Emoji (Noto Color) — fallback for proportional + dedicated family.
    if let Some(data) = load_first(&[
        "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    ]) {
        fonts
            .font_data
            .insert("emoji".into(), Arc::new(FontData::from_owned(data)));
        fonts.families.insert(
            FontFamily::Name("emoji".into()),
            vec!["emoji".into()],
        );
        for fam in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name("serif".into()),
        ] {
            if let Some(list) = fonts.families.get_mut(&fam) {
                list.push("emoji".into());
            }
        }
    }

    ctx.set_fonts(fonts);
}

/// Fichier de la mono système — le même alias `monospace` que le terminal Omarchy.
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

pub fn emoji_font() -> FontFamily {
    FontFamily::Name("emoji".into())
}

fn load_first(paths: &[&str]) -> Option<Vec<u8>> {
    for p in paths {
        if let Ok(b) = std::fs::read(p) {
            return Some(b);
        }
    }
    None
}
