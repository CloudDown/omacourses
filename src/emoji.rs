//! Icônes de dossier. Noto Color Emoji est une bitmap CBDT (PNG), sans contours :
//! egui ne la rastérise pas. On catalogue tous les glyphes et on décode à la demande.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

const TEX_SIDE: u32 = 72;

pub struct Glyph {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct Atlas {
    font: Option<FontBytes>,
    glyphs: HashMap<char, Glyph>,
    /// Tous les emojis colorés disponibles, ordre Unicode.
    catalog: Vec<char>,
}

impl Atlas {
    pub fn load() -> Self {
        let mut atlas = Self {
            font: None,
            glyphs: HashMap::new(),
            catalog: Vec::new(),
        };
        let Some(data) = color_emoji_bytes() else {
            return atlas;
        };
        let Some(font) = FontBytes::parse(data) else {
            return atlas;
        };
        atlas.catalog = font.list_chars();
        atlas.font = Some(font);
        atlas
    }

    pub fn catalog(&self) -> &[char] {
        &self.catalog
    }

    pub fn ensure(&mut self, ch: char) -> Option<&Glyph> {
        if self.glyphs.contains_key(&ch) {
            return self.glyphs.get(&ch);
        }
        let g = self.font.as_ref()?.decode(ch)?;
        self.glyphs.insert(ch, g);
        self.glyphs.get(&ch)
    }

    #[allow(dead_code)]
    pub fn ensure_str(&mut self, emoji: &str) -> Option<&Glyph> {
        let ch = self.key(emoji)?;
        self.ensure(ch)
    }

    pub fn key(&self, emoji: &str) -> Option<char> {
        emoji.chars().find(|ch| {
            *ch != '\u{fe0f}'
                && *ch != '\u{fe0e}'
                && *ch != '\u{200d}'
                && self.catalog.contains(ch)
        })
    }

    #[allow(dead_code)]
    pub fn get(&self, emoji: &str) -> Option<&Glyph> {
        self.key(emoji).and_then(|ch| self.glyphs.get(&ch))
    }
}

struct Group {
    start: u32,
    end: u32,
    glyph: u32,
}

struct FontBytes {
    data: Vec<u8>,
    groups: Vec<Group>,
    cblc: (usize, usize),
    cbdt: (usize, usize),
}

impl FontBytes {
    fn parse(data: Vec<u8>) -> Option<Self> {
        let tables = tables(&data)?;
        let cmap = slice(&data, *tables.get("cmap")?)?;
        let cblc = *tables.get("CBLC")?;
        let cbdt = *tables.get("CBDT")?;
        Some(Self {
            groups: cmap_groups(cmap)?,
            data,
            cblc,
            cbdt,
        })
    }

    fn cblc(&self) -> &[u8] {
        &self.data[self.cblc.0..self.cblc.0 + self.cblc.1]
    }

    fn cbdt(&self) -> &[u8] {
        &self.data[self.cbdt.0..self.cbdt.0 + self.cbdt.1]
    }

    fn list_chars(&self) -> Vec<char> {
        let mut out = Vec::new();
        for g in &self.groups {
            for cp in g.start..=g.end {
                if !usable(cp) {
                    continue;
                }
                let Some(ch) = char::from_u32(cp) else {
                    continue;
                };
                let Some(gid) = glyph_id(&self.groups, cp) else {
                    continue;
                };
                if locate(self.cblc(), gid).is_some() {
                    out.push(ch);
                }
            }
        }
        out
    }

    fn decode(&self, ch: char) -> Option<Glyph> {
        let gid = glyph_id(&self.groups, ch as u32)?;
        let (off, len) = locate(self.cblc(), gid)?;
        let blob = self.cbdt().get(off..off + len)?;
        let dlen = read_u32(blob, 5)? as usize;
        let png = blob.get(9..9 + dlen)?;
        let img = image::load_from_memory(png).ok()?.into_rgba8();
        let (w, h) = img.dimensions();
        let img = if w > TEX_SIDE || h > TEX_SIDE {
            let s = (TEX_SIDE as f32 / w.max(h) as f32).min(1.0);
            let nw = ((w as f32 * s).round() as u32).max(1);
            let nh = ((h as f32 * s).round() as u32).max(1);
            image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        Some(Glyph {
            width: img.width() as usize,
            height: img.height() as usize,
            rgba: img.into_raw(),
        })
    }
}

fn usable(cp: u32) -> bool {
    // Variation selectors, ZWJ, tags — pas des icônes seuls.
    if matches!(cp, 0x200D | 0xFE0E | 0xFE0F | 0x20E3) {
        return false;
    }
    if (0xE0020..=0xE007F).contains(&cp) {
        return false;
    }
    // Privé / tags régionaux seuls peu utiles en grille.
    if (0xE000..=0xF8FF).contains(&cp) {
        return false;
    }
    // Assez haut pour les symboles, ou bloc dingbat / emoji.
    cp >= 0x00A9 || (0x203C..=0x3299).contains(&cp)
}

fn color_emoji_bytes() -> Option<Vec<u8>> {
    for p in [
        "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    ] {
        if let Ok(b) = std::fs::read(p) {
            return Some(b);
        }
    }
    let out = Command::new("fc-match")
        .args(["Noto Color Emoji", "-f", "%{file}\n"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        None
    } else {
        std::fs::read(PathBuf::from(p)).ok()
    }
}

fn tables(data: &[u8]) -> Option<HashMap<&str, (usize, usize)>> {
    if data.len() < 12 || data.get(0..4) != Some(&[0, 1, 0, 0]) {
        return None;
    }
    let n = read_u16(data, 4)? as usize;
    let mut map = HashMap::new();
    for i in 0..n {
        let at = 12 + i * 16;
        let tag = std::str::from_utf8(data.get(at..at + 4)?).ok()?;
        let off = read_u32(data, at + 8)? as usize;
        let len = read_u32(data, at + 12)? as usize;
        map.insert(tag, (off, len));
    }
    Some(map)
}

fn slice(data: &[u8], span: (usize, usize)) -> Option<&[u8]> {
    data.get(span.0..span.0 + span.1)
}

fn cmap_groups(cmap: &[u8]) -> Option<Vec<Group>> {
    let n = read_u16(cmap, 2)? as usize;
    for i in 0..n {
        let at = 4 + i * 8;
        let sub = read_u32(cmap, at + 4)? as usize;
        if read_u16(cmap, sub)? != 12 {
            continue;
        }
        let ng = read_u32(cmap, sub + 12)? as usize;
        let mut groups = Vec::with_capacity(ng);
        for g in 0..ng {
            let o = sub + 16 + g * 12;
            groups.push(Group {
                start: read_u32(cmap, o)?,
                end: read_u32(cmap, o + 4)?,
                glyph: read_u32(cmap, o + 8)?,
            });
        }
        return Some(groups);
    }
    None
}

fn glyph_id(groups: &[Group], cp: u32) -> Option<u16> {
    let i = groups.partition_point(|g| g.end < cp);
    let g = groups.get(i)?;
    if cp < g.start || cp > g.end {
        return None;
    }
    u16::try_from(g.glyph + (cp - g.start)).ok()
}

fn locate(cblc: &[u8], gid: u16) -> Option<(usize, usize)> {
    let nsub = read_u32(cblc, 16)? as usize;
    let arr = read_u32(cblc, 8)? as usize;
    for i in 0..nsub {
        let ent = arr + i * 8;
        let first = read_u16(cblc, ent)?;
        let last = read_u16(cblc, ent + 2)?;
        if gid < first || gid > last {
            continue;
        }
        let base = arr + read_u32(cblc, ent + 4)? as usize;
        if read_u16(cblc, base)? != 1 || read_u16(cblc, base + 2)? != 17 {
            return None;
        }
        let image_off = read_u32(cblc, base + 4)? as usize;
        let idx = (gid - first) as usize;
        let o1 = read_u32(cblc, base + 8 + idx * 4)? as usize;
        let o2 = read_u32(cblc, base + 12 + idx * 4)? as usize;
        if o2 < o1 {
            return None;
        }
        return Some((image_off + o1, o2 - o1));
    }
    None
}

fn read_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Favoris affichés en tête de casse.
pub const FAVORITES: &[&str] = &[
    "📝", "📕", "📗", "📘", "📙", "📒", "📓", "✨", "💡", "🎯", "⭐", "🔥", "🌙", "☕", "🎵",
    "📐", "🧪", "🧠", "💼", "🗂️", "📌", "🖤", "🌿", "🚀", "💎", "🔮",
];

#[cfg(test)]
mod tests {
    use super::Atlas;

    #[test]
    fn color_emoji_catalog() {
        let mut atlas = Atlas::load();
        if atlas.catalog().is_empty() {
            return;
        }
        assert!(atlas.catalog().len() > 200);
        let g = atlas.ensure_str("📝").expect("memo");
        assert!(g.width >= 24 && g.height >= 24);
    }
}
