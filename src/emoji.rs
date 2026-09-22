//! Icônes de dossier. Noto Color Emoji est une bitmap CBDT (PNG), sans contours :
//! egui ne la rastérise pas, les glyphes sortent vides. On extrait les PNG nous-mêmes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

pub struct Glyph {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct Atlas {
    glyphs: HashMap<char, Glyph>,
}

impl Atlas {
    pub fn load(wanted: &[&str]) -> Self {
        let mut glyphs = HashMap::new();
        let Some(bytes) = color_emoji_bytes() else {
            return Self { glyphs };
        };
        let Some(font) = Font::parse(&bytes) else {
            return Self { glyphs };
        };
        for em in wanted {
            let Some(ch) = em.chars().find(|c| *c != '\u{fe0f}' && *c != '\u{200d}') else {
                continue;
            };
            if glyphs.contains_key(&ch) {
                continue;
            }
            if let Some(g) = font.decode(ch) {
                glyphs.insert(ch, g);
            }
        }
        Self { glyphs }
    }

    pub fn get(&self, emoji: &str) -> Option<&Glyph> {
        self.key(emoji).and_then(|ch| self.glyphs.get(&ch))
    }

    pub fn key(&self, emoji: &str) -> Option<char> {
        emoji.chars().find(|ch| self.glyphs.contains_key(ch))
    }
}

struct Group {
    start: u32,
    end: u32,
    glyph: u32,
}

struct Font<'a> {
    groups: Vec<Group>,
    cblc: &'a [u8],
    cbdt: &'a [u8],
}

impl<'a> Font<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let tables = tables(data)?;
        let cmap = slice(data, tables.get("cmap")?)?;
        let cblc = slice(data, tables.get("CBLC")?)?;
        let cbdt = slice(data, tables.get("CBDT")?)?;
        Some(Self {
            groups: cmap_groups(cmap)?,
            cblc,
            cbdt,
        })
    }

    fn decode(&self, ch: char) -> Option<Glyph> {
        let gid = glyph_id(&self.groups, ch as u32)?;
        let (off, len) = locate(self.cblc, gid)?;
        let blob = self.cbdt.get(off..off + len)?;
        // Image format 17 : métriques (5) + longueur + PNG.
        let dlen = read_u32(blob, 5)? as usize;
        let png = blob.get(9..9 + dlen)?;
        let img = image::load_from_memory(png).ok()?.into_rgba8();
        Some(Glyph {
            width: img.width() as usize,
            height: img.height() as usize,
            rgba: img.into_raw(),
        })
    }
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

fn slice<'a>(data: &'a [u8], span: &(usize, usize)) -> Option<&'a [u8]> {
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

/// Offset et longueur dans CBDT pour un glyphe (index format 1, image PNG).
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

#[cfg(test)]
mod tests {
    use super::Atlas;

    #[test]
    fn color_emoji_decodes() {
        let atlas = Atlas::load(&["📝", "🧪", "🗂️"]);
        if atlas.get("📝").is_none() {
            return;
        }
        for em in ["📝", "🧪", "🗂️"] {
            let g = atlas.get(em).unwrap_or_else(|| panic!("{em}"));
            assert!(
                g.width >= 32 && g.height >= 32,
                "{em} {}x{}",
                g.width,
                g.height
            );
            assert_eq!(g.rgba.len(), g.width * g.height * 4);
            assert!(
                g.rgba
                    .chunks(4)
                    .any(|px| px[3] > 200 && (px[0] > 8 || px[1] > 8 || px[2] > 8)),
                "{em} entièrement transparent"
            );
        }
    }
}
