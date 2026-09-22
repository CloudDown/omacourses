use std::fs;
use std::path::{Path, PathBuf};

use crate::document::Note;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: Uuid,
    pub title: String,
    pub updated: DateTime<Utc>,
    pub pinned: bool,
    pub cover: u8,
    #[serde(default)]
    pub emoji: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl DockEdge {
    pub fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// Posture du pupitre : clavier+souris, ou stylet+main.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteMode {
    /// Souris écrit, clavier pilote, espace panorama.
    #[default]
    #[serde(alias = "main")]
    Pupitre,
    /// Stylet écrit, doigt pousse la feuille.
    #[serde(alias = "stylus", alias = "chiffon")]
    Tablette,
}

impl NoteMode {
    pub fn other(self) -> Self {
        match self {
            Self::Pupitre => Self::Tablette,
            Self::Tablette => Self::Pupitre,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pupitre => "Desktop",
            Self::Tablette => "Tablet",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Pupitre => "Desktop · mouse & keyboard",
            Self::Tablette => "Tablet · stylus & palm",
        }
    }

    pub fn is_tablette(self) -> bool {
        matches!(self, Self::Tablette)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub seeded: bool,
    pub notes: Vec<NoteMeta>,
    #[serde(default)]
    pub dock: DockEdge,
    #[serde(default, alias = "hand")]
    pub mode: NoteMode,
    /// `true` = tuto fermé (bouton seul).
    #[serde(default = "default_true")]
    pub fiche_pliee: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Index {
    fn default() -> Self {
        Self {
            seeded: false,
            notes: Vec::new(),
            dock: DockEdge::default(),
            mode: NoteMode::default(),
            fiche_pliee: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Library {
    pub root: PathBuf,
    pub index: Index,
}

impl Library {
    pub fn open() -> Self {
        let root = data_dir();
        let _ = fs::create_dir_all(root.join("notes"));
        let index_path = root.join("library.json");
        let index = fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { root, index }
    }

    pub fn save_index(&self) {
        let _ = fs::create_dir_all(&self.root);
        if let Ok(s) = serde_json::to_string_pretty(&self.index) {
            let _ = fs::write(self.root.join("library.json"), s);
        }
    }

    pub fn note_dir(&self, id: Uuid) -> PathBuf {
        self.root.join("notes").join(id.to_string())
    }

    pub fn load_note(&self, id: Uuid) -> Option<Note> {
        let path = self.note_dir(id).join("note.json");
        let s = fs::read_to_string(path).ok()?;
        serde_json::from_str(&s).ok()
    }

    pub fn save_note(&mut self, note: &Note) {
        let dir = self.note_dir(note.id);
        let _ = fs::create_dir_all(dir.join("media"));
        if let Ok(s) = serde_json::to_string(&note) {
            let _ = fs::write(dir.join("note.json"), s);
        }
        if let Some(meta) = self.index.notes.iter_mut().find(|m| m.id == note.id) {
            meta.title = note.title.clone();
            meta.updated = note.updated;
            meta.pinned = note.pinned;
            meta.cover = note.cover;
            meta.emoji = note.emoji.clone();
        } else {
            self.index.notes.push(NoteMeta {
                id: note.id,
                title: note.title.clone(),
                updated: note.updated,
                pinned: note.pinned,
                cover: note.cover,
                emoji: note.emoji.clone(),
            });
        }
        self.index
            .notes
            .sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated.cmp(&a.updated)));
        self.save_index();
    }

    pub fn insert_new(&mut self, note: &Note) {
        self.save_note(note);
    }

    pub fn delete_note(&mut self, id: Uuid) {
        self.index.notes.retain(|m| m.id != id);
        let _ = fs::remove_dir_all(self.note_dir(id));
        self.save_index();
    }

    pub fn duplicate(&mut self, id: Uuid) -> Option<Note> {
        let mut note = self.load_note(id)?;
        note.id = Uuid::new_v4();
        note.title = format!("{} (copy)", note.title);
        note.touch();
        for page in &mut note.pages {
            for s in &mut page.strokes {
                s.id = Uuid::new_v4();
            }
            for t in &mut page.texts {
                t.id = Uuid::new_v4();
            }
            for im in &mut page.images {
                let old = im.file.clone();
                im.id = Uuid::new_v4();
                im.file = format!("{}.png", im.id);
                let src = self.note_dir(id).join("media").join(&old);
                let dst = self.note_dir(note.id).join("media");
                let _ = fs::create_dir_all(&dst);
                let _ = fs::copy(src, dst.join(&im.file));
            }
        }
        self.save_note(&note);
        Some(note)
    }

    pub fn write_media(&self, note_id: Uuid, bytes: &[u8]) -> Option<String> {
        let id = Uuid::new_v4();
        let name = format!("{id}.png");
        let dir = self.note_dir(note_id).join("media");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join(&name), bytes).ok()?;
        Some(name)
    }

    pub fn media_path(&self, note_id: Uuid, file: &str) -> PathBuf {
        self.note_dir(note_id).join("media").join(file)
    }

    pub fn mark_seeded(&mut self) {
        self.index.seeded = true;
        self.save_index();
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("CAHIER_DATA") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("com", "clouddown", "omacourses")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".").join("data"))
}

pub fn ensure_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.into_rgba8();
    let (mut w, mut h) = img.dimensions();
    let max = 2048u32;
    let rgba = if w > max || h > max {
        let s = (max as f32 / w.max(h) as f32).min(1.0);
        w = (w as f32 * s) as u32;
        h = (h as f32 * s) as u32;
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

pub fn image_size(path: &Path) -> Option<(f32, f32)> {
    let img = image::open(path).ok()?;
    Some((img.width() as f32, img.height() as f32))
}
