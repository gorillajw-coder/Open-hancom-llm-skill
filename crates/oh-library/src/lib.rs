//! Library loading: `manifest.toml` lists every shipped file with its sha256; loading
//! verifies each hash before anything is parsed. Searching and anonymizing are P5; only a
//! minimal search interface exists here.
use oh_recipe::{hex_sha256, Card, Defaults, Engine, Frame, Regulation};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

mod stack;
pub use stack::{open_pack, read_sockets, valid_pack_name, write_sockets, ZipSource, Conflict, LayerInfo, PackEntry, Stack, StackHit, PERSONAL_DIR, SOCKETS_FILE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibError {
    pub code: &'static str,
    pub detail: String,
}
impl std::fmt::Display for LibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl std::error::Error for LibError {}
fn err(code: &'static str, detail: impl Into<String>) -> LibError {
    LibError { code, detail: detail.into() }
}
pub type Result<T> = std::result::Result<T, LibError>;

pub const MAX_FILE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    /// The shipped library and shared packs hold anonymized material only (a personal
    /// library may hold real values, D12).
    pub anonymized: bool,
    /// Search is not neural; P5 builds the real index.
    #[serde(default)]
    pub search: String,
    /// Pack name suggested to `oh library add` (optional).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "file")]
    pub files: Vec<FileEntry>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub kind: String,
}

/// Where library bytes come from. `DirSource` reads a folder via oh-host.
pub trait Source {
    fn read(&self, rel: &str) -> Result<Vec<u8>>;
}
pub struct DirSource {
    root: PathBuf,
}
impl DirSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}
fn safe_rel(rel: &str) -> bool {
    !rel.is_empty() && !rel.starts_with('/') && !rel.contains('\\') && !rel.contains(':') && rel.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
impl Source for DirSource {
    fn read(&self, rel: &str) -> Result<Vec<u8>> {
        if !safe_rel(rel) {
            return Err(err("unsafe_path", rel));
        }
        oh_host::bounded_read(&self.root.join(rel), MAX_FILE).map_err(|c| err("library_io_error", format!("{rel}: {c}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
}

pub struct Library {
    pub manifest: Manifest,
    /// sha256 of `manifest.toml` itself: a pack is pinned to it when it is connected.
    pub manifest_sha256: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl Library {
    pub fn load(src: &dyn Source) -> Result<Library> {
        Self::load_opts(src, true)
    }
    /// `require_anonymized = false` only for the user's own personal library (D12).
    pub fn load_opts(src: &dyn Source, require_anonymized: bool) -> Result<Library> {
        let raw = src.read("manifest.toml")?;
        let manifest_sha256 = hex_sha256(&raw);
        let text = String::from_utf8(raw).map_err(|_| err("invalid_manifest", "not utf-8"))?;
        let manifest: Manifest = toml::from_str(&text).map_err(|e| err("invalid_manifest", e.to_string()))?;
        if require_anonymized && !manifest.anonymized {
            return Err(err("library_not_anonymized", "manifest.anonymized must be true for a shipped library"));
        }
        let mut files = BTreeMap::new();
        for f in &manifest.files {
            if f.path == "manifest.toml" || files.contains_key(&f.path) {
                return Err(err("invalid_manifest", format!("duplicate or self entry {}", f.path)));
            }
            let bytes = src.read(&f.path)?;
            if hex_sha256(&bytes) != f.sha256 {
                return Err(err("manifest_hash_mismatch", f.path.clone()));
            }
            files.insert(f.path.clone(), bytes);
        }
        Ok(Library { manifest, manifest_sha256, files })
    }
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
    /// Regulation tables (`kind = "regulation_table"`).
    pub fn regulation_tables(&self) -> Result<Vec<(String, Regulation)>> {
        let mut out = Vec::new();
        for f in self.manifest.files.iter().filter(|f| f.kind == "regulation_table") {
            let s = std::str::from_utf8(&self.files[&f.path]).map_err(|_| err("invalid_regulation", f.path.clone()))?;
            out.push((f.path.clone(), Regulation::from_toml(s).map_err(|e| err("invalid_regulation", format!("{}: {e}", f.path)))?));
        }
        Ok(out)
    }
    /// `defaults.toml` when the manifest lists it (a pack may leave it out).
    pub fn defaults_opt(&self) -> Result<Option<Defaults>> {
        match self.file("defaults.toml") {
            None => Ok(None),
            Some(_) => self.defaults().map(Some),
        }
    }
    pub fn defaults(&self) -> Result<Defaults> {
        let b = self.file("defaults.toml").ok_or_else(|| err("missing_file", "defaults.toml"))?;
        let s = std::str::from_utf8(b).map_err(|_| err("invalid_defaults", "not utf-8"))?;
        Defaults::from_toml(s).map_err(|e| err("invalid_defaults", e.to_string()))
    }
    /// Common frames (`frames/*.toml`, kind `frame`) that cards inherit with `extends`.
    pub fn frames(&self) -> Result<Vec<Frame>> {
        let mut out: Vec<Frame> = Vec::new();
        for f in self.manifest.files.iter().filter(|f| f.kind == "frame") {
            let s = std::str::from_utf8(&self.files[&f.path]).map_err(|_| err("invalid_frame", f.path.clone()))?;
            let fr = Frame::from_toml(s).map_err(|e| err("invalid_frame", format!("{}: {e}", f.path)))?;
            if out.iter().any(|x| x.id == fr.id) {
                return Err(err("invalid_frame", format!("duplicate frame id {}", fr.id)));
            }
            out.push(fr);
        }
        Ok(out)
    }
    /// Cards with their template bytes (template must be a manifest-listed file), frames merged.
    pub fn cards(&self) -> Result<Vec<(Card, Vec<u8>)>> {
        self.cards_with(&self.frames()?)
    }
    /// Cards resolved against the given frames (a pack card may extend a base frame).
    pub fn cards_with(&self, frames: &[Frame]) -> Result<Vec<(Card, Vec<u8>)>> {
        let mut out = Vec::new();
        for f in self.manifest.files.iter().filter(|f| f.kind == "card") {
            let s = std::str::from_utf8(&self.files[&f.path]).map_err(|_| err("invalid_card", f.path.clone()))?;
            let card = Card::from_toml(s).and_then(|c| c.with_frames(frames)).map_err(|e| err("invalid_card", format!("{}: {e}", f.path)))?;
            let t = self.files.get(&card.template).ok_or_else(|| err("missing_file", format!("template {} of card {}", card.template, card.id)))?;
            out.push((card, t.clone()));
        }
        Ok(out)
    }
    pub fn engine(&self) -> Result<Engine> {
        Ok(self.engine_with_local(None)?.0)
    }
    /// Engine with shipped regulation tables, then a company table from `local_dir`
    /// (`<id>.toml`, status `company`) replacing the shipped sample of the same id.
    /// Returns notes on which tables are in use.
    pub fn engine_with_local(&self, local_dir: Option<&std::path::Path>) -> Result<(Engine, Vec<String>)> {
        let mut e = Engine::new(self.defaults()?, self.cards()?).map_err(|e| err("card_rejected", e.to_string()))?;
        let mut notes = Vec::new();
        for (path, r) in self.regulation_tables()? {
            set_regulation_with_local(&mut e, &mut notes, &path, r, local_dir);
        }
        for (c, _) in self.cards()? {
            if let Some(x) = &c.xlsx {
                let b = self.files.get(&x.template).ok_or_else(|| err("missing_file", format!("xlsx template {} of card {}", x.template, c.id)))?;
                e.attach_xlsx_template(&c.id, b.clone()).map_err(|e| err("card_rejected", e.to_string()))?;
            }
        }
        Ok((e, notes))
    }
    /// Minimal P3 interface: substring match on card id, title and triggers.
    pub fn search(&self, query: &str) -> Result<Vec<Hit>> {
        let q = query.trim();
        let mut hits: Vec<(bool, Hit)> = self
            .cards()?
            .into_iter()
            .filter(|(c, _)| !q.is_empty() && (c.id.contains(q) || c.title.contains(q) || c.triggers.iter().any(|t| t.contains(q) || q.contains(t.as_str()))))
            .map(|(c, _)| (c.fallback, Hit { kind: "card", id: c.id, title: c.title }))
            .collect();
        hits.sort_by_key(|(f, _)| *f); // fallback cards last (stable)
        Ok(hits.into_iter().map(|(_, h)| h).collect())
    }
}

/// Install a regulation table; a company table `<local_dir>/<id>.toml` (status `company`)
/// replaces it. A broken company table must not take every card down: the shipped sample
/// stays (its rules warn) and the note says why.
pub(crate) fn set_regulation_with_local(e: &mut Engine, notes: &mut Vec<String>, label: &str, r: Regulation, local_dir: Option<&std::path::Path>) {
    let local = local_dir.map(|d| d.join(format!("{}.toml", r.id))).filter(|p| p.is_file());
    match local {
        Some(p) => {
            let read = oh_host::bounded_read(&p, MAX_FILE)
                .map_err(|c| c.to_string())
                .and_then(|b| String::from_utf8(b).map_err(|_| "not utf-8".to_string()))
                .and_then(|s| Regulation::from_toml(&s).map_err(|e| e.to_string()))
                .and_then(|lr| if lr.id == r.id && lr.status == "company" { Ok(lr) } else { Err(format!("id {} / status company 필요", r.id)) });
            match read {
                Ok(lr) => {
                    notes.push(format!("{}: 회사표 {}", r.id, p.display()));
                    e.set_regulation(lr);
                }
                Err(why) => {
                    notes.push(format!("{}: 회사표 무시({}: {why}) → 라이브러리 {} ({})", r.id, p.display(), label, r.status));
                    e.set_regulation(r);
                }
            }
        }
        None => {
            notes.push(format!("{}: 라이브러리 {} ({})", r.id, label, r.status));
            e.set_regulation(r);
        }
    }
}
