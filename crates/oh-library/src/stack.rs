//! Library socket (D12): several library layers loaded at once, in priority order
//! personal > connected shared packs (in connection order) > base (shipped `library/`).
//!
//! * A layer is a folder or a `.zip` with `manifest.toml`; every listed file is hash-checked
//!   (`Library::load`). Shared packs must be anonymized; the personal library need not be.
//! * A connected pack is also pinned to the sha256 of its `manifest.toml` at connection time:
//!   a pack that changed afterwards is not loaded until it is connected again.
//! * The same card / frame / regulation-table id / default key in several layers: the
//!   higher layer wins and the overlap is reported as a conflict.
//! * A layer that fails (missing, hash mismatch, bad card) is skipped and reported; only a
//!   broken base library makes the whole stack fail.
//! * Connections live in `<install root>/sockets.toml`; the personal library is
//!   `<install root>/personal-library/`. The install root is the folder holding `library/`.
use crate::{err, set_regulation_with_local, DirSource, Library, Result, Source, MAX_FILE};
use oh_recipe::{Card, Defaults, Engine, Frame, Regulation};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const SOCKETS_FILE: &str = "sockets.toml";
pub const PERSONAL_DIR: &str = "personal-library";
/// A zipped pack may not exceed this (each member is still capped at `MAX_FILE`).
pub const MAX_PACK: u64 = 256 * 1024 * 1024;

/// `[[pack]]` row of `sockets.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackEntry {
    pub name: String,
    pub path: PathBuf,
    /// sha256 of the pack's manifest.toml when it was connected.
    pub manifest_sha256: String,
    #[serde(default)]
    pub added: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sockets {
    #[serde(default, rename = "pack")]
    packs: Vec<PackEntry>,
}

/// Pack names: 1–40 letters, digits, `-_.`, not starting with `.`; `base`/`personal` are layer names.
pub fn valid_pack_name(n: &str) -> bool {
    let len = n.chars().count();
    (1..=40).contains(&len) && !["base", "personal", SOCKETS_FILE].contains(&n) && n.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c)) && !n.starts_with('.')
}

pub fn read_sockets(root: &Path) -> Result<Vec<PackEntry>> {
    let p = root.join(SOCKETS_FILE);
    if !p.exists() {
        return Ok(vec![]);
    }
    let b = oh_host::bounded_read(&p, 1024 * 1024).map_err(|c| err("sockets_invalid", format!("{}: {c}", p.display())))?;
    let s = String::from_utf8(b).map_err(|_| err("sockets_invalid", "not utf-8"))?;
    let v: Sockets = toml::from_str(&s).map_err(|e| err("sockets_invalid", e.to_string()))?;
    let mut names: Vec<&str> = Vec::new();
    for pk in &v.packs {
        if !valid_pack_name(&pk.name) {
            return Err(err("sockets_invalid", format!("invalid pack name {:?}", pk.name)));
        }
        if names.contains(&pk.name.as_str()) {
            return Err(err("sockets_invalid", format!("duplicate pack name {}", pk.name)));
        }
        names.push(&pk.name);
    }
    Ok(v.packs)
}
fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
/// Replace `sockets.toml` atomically.
pub fn write_sockets(root: &Path, packs: &[PackEntry]) -> Result<()> {
    let mut out = String::from("# Open-hancom 공용 라이브러리 연결 (oh library add/remove가 관리한다. 우선순위: 개인 > 아래 순서 > 기본)\n");
    for p in packs {
        let path = p.path.to_str().ok_or_else(|| err("invalid_arguments", "path is not unicode"))?;
        out.push_str(&format!("\n[[pack]]\nname = {}\npath = {}\nmanifest_sha256 = {}\nadded = {}\n", toml_str(&p.name), toml_str(path), toml_str(&p.manifest_sha256), toml_str(&p.added)));
    }
    oh_host::ensure_dir(root).map_err(|c| err("sockets_write_failed", c))?;
    oh_host::replace_file(&root.join(SOCKETS_FILE), out.as_bytes()).map_err(|c| err("sockets_write_failed", c))
}

/// Files of a zipped pack: `manifest.toml` at the root of the zip or inside its single top folder.
/// Members are decompressed lazily, only when the loader asks for a manifest-listed path, each
/// capped at `MAX_FILE` and all together at `MAX_PACK` (unlisted members are never inflated).
pub struct ZipSource {
    /// Parsed once; members are inflated on demand.
    zip: std::cell::RefCell<zip::ZipArchive<std::io::Cursor<Vec<u8>>>>,
    prefix: String,
    inflated: std::cell::Cell<u64>,
}
/// More entries than this is not a library pack.
pub const MAX_ZIP_ENTRIES: usize = 20_000;
impl ZipSource {
    pub fn open(path: &Path) -> Result<ZipSource> {
        let bytes = oh_host::bounded_read(path, MAX_PACK).map_err(|c| err("library_io_error", format!("{}: {c}", path.display())))?;
        let z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| err("invalid_pack", format!("zip: {e}")))?;
        if z.len() > MAX_ZIP_ENTRIES {
            return Err(err("invalid_pack", format!("zip has {} entries", z.len())));
        }
        let names: Vec<String> = z.file_names().map(String::from).collect();
        let prefix = if names.iter().any(|n| n == "manifest.toml") {
            String::new()
        } else {
            let tops: Vec<&String> = names.iter().filter(|n| n.ends_with("/manifest.toml") && n.matches('/').count() == 1).collect();
            match tops.as_slice() {
                [one] => one.trim_end_matches("manifest.toml").to_string(),
                _ => return Err(err("invalid_pack", "zip has no manifest.toml at its root or in one top folder")),
            }
        };
        Ok(ZipSource { zip: std::cell::RefCell::new(z), prefix, inflated: std::cell::Cell::new(0) })
    }
}
impl Source for ZipSource {
    fn read(&self, rel: &str) -> Result<Vec<u8>> {
        if !crate::safe_rel(rel) {
            return Err(err("unsafe_path", rel));
        }
        let mut z = self.zip.borrow_mut();
        let mut f = z.by_name(&format!("{}{rel}", self.prefix)).map_err(|_| err("library_io_error", format!("{rel}: not in zip")))?;
        if !f.is_file() {
            return Err(err("library_io_error", format!("{rel}: not a file")));
        }
        let mut buf = Vec::new();
        (&mut f).take(MAX_FILE + 1).read_to_end(&mut buf).map_err(|e| err("invalid_pack", format!("{rel}: {e}")))?;
        if buf.len() as u64 > MAX_FILE {
            return Err(err("invalid_pack", format!("{rel}: too large")));
        }
        let total = self.inflated.get() + buf.len() as u64;
        if total > MAX_PACK {
            return Err(err("invalid_pack", "pack contents exceed the size limit"));
        }
        self.inflated.set(total);
        Ok(buf)
    }
}

/// Open a pack (folder or .zip) with full hash checks. Shared packs must be anonymized.
pub fn open_pack(path: &Path, require_anonymized: bool) -> Result<Library> {
    if path.is_dir() {
        Library::load_opts(&DirSource::new(path), require_anonymized)
    } else if path.is_file() {
        let is_zip = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if !is_zip {
            return Err(err("invalid_pack", format!("{}: folder or .zip only", path.display())));
        }
        Library::load_opts(&ZipSource::open(path)?, require_anonymized)
    } else {
        Err(err("pack_not_found", path.display().to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerInfo {
    /// `personal`, the pack name, or `base`.
    pub name: String,
    /// `personal` | `pack` | `base`
    pub kind: &'static str,
    pub path: PathBuf,
    pub version: Option<String>,
    /// `loaded`, or why it was skipped (`code: detail`).
    pub status: String,
    /// Card ids this layer supplies that are in use (not shadowed by a higher layer).
    pub cards: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// `card` | `frame` | `regulation_table` | `default`
    pub kind: &'static str,
    pub id: String,
    pub used: String,
    pub shadowed: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackHit {
    pub id: String,
    pub title: String,
    pub layer: String,
    /// Generic fallback card: callers list it after specific cards.
    pub fallback: bool,
    /// The card's id or one of its triggers appears whole in the query (not just a shared word
    /// of its title, or a short query word inside a longer trigger).
    pub strong: bool,
}

pub struct Stack {
    /// All layers in priority order (highest first), loaded or not.
    pub layers: Vec<LayerInfo>,
    /// Loaded libraries in priority order, with their layer name.
    libs: Vec<(String, Library)>,
    pub conflicts: Vec<Conflict>,
    /// card id → layer name that supplies it
    card_layer: BTreeMap<String, String>,
}

struct Merged {
    defaults: Defaults,
    cards: Vec<(Card, Vec<u8>, String)>,
    regs: Vec<(String, Regulation, String)>,
    conflicts: Vec<Conflict>,
}

fn note(conflicts: &mut Vec<Conflict>, kind: &'static str, id: &str, used: &str, other: &str) {
    match conflicts.iter_mut().find(|c| c.kind == kind && c.id == id) {
        Some(c) => c.shadowed.push(other.to_string()),
        None => conflicts.push(Conflict { kind, id: id.to_string(), used: used.to_string(), shadowed: vec![other.to_string()] }),
    }
}

/// Merge libraries given highest priority first. The last one is the base and must have defaults.
fn merge(libs: &[(&str, &Library)]) -> Result<Merged> {
    let mut conflicts = Vec::new();
    let mut frames: Vec<(Frame, String)> = Vec::new();
    for (name, lib) in libs {
        for f in lib.frames()? {
            match frames.iter().find(|(x, _)| x.id == f.id) {
                Some((_, used)) => note(&mut conflicts, "frame", &f.id, &used.clone(), name),
                None => frames.push((f, name.to_string())),
            }
        }
    }
    let fr: Vec<Frame> = frames.into_iter().map(|(f, _)| f).collect();
    let mut cards: Vec<(Card, Vec<u8>, String)> = Vec::new();
    let mut regs: Vec<(String, Regulation, String)> = Vec::new();
    for (name, lib) in libs {
        for (c, t) in lib.cards_with(&fr)? {
            match cards.iter().find(|(x, _, _)| x.id == c.id) {
                Some((_, _, used)) => note(&mut conflicts, "card", &c.id, &used.clone(), name),
                None => cards.push((c, t, name.to_string())),
            }
        }
        for (path, r) in lib.regulation_tables()? {
            match regs.iter().find(|(_, x, _)| x.id == r.id) {
                Some((_, _, used)) => note(&mut conflicts, "regulation_table", &r.id, &used.clone(), name),
                None => regs.push((format!("{name}:{path}"), r, name.to_string())),
            }
        }
    }
    // defaults: base first, higher layers override per key
    let (base_name, base) = libs.last().ok_or_else(|| err("library_unavailable", "no base"))?;
    let mut defaults = base.defaults()?;
    let mut owner: BTreeMap<String, String> = defaults.defaults.keys().map(|k| (k.clone(), base_name.to_string())).collect();
    for (name, lib) in libs[..libs.len() - 1].iter().rev() {
        if let Some(d) = lib.defaults_opt()? {
            for (k, v) in d.defaults {
                if let Some(prev) = owner.insert(k.clone(), name.to_string()) {
                    // reported from the winner's side: lower layers are shadowed
                    match conflicts.iter_mut().find(|c| c.kind == "default" && c.id == k) {
                        Some(c) => {
                            c.shadowed.insert(0, c.used.clone());
                            c.used = name.to_string();
                        }
                        None => conflicts.push(Conflict { kind: "default", id: k.clone(), used: name.to_string(), shadowed: vec![prev] }),
                    }
                }
                defaults.defaults.insert(k, v);
            }
        }
    }
    Ok(Merged { defaults, cards, regs, conflicts })
}

fn build_engine(libs: &[(&str, &Library)], local_dir: Option<&Path>) -> Result<(Engine, Vec<String>, Merged)> {
    let m = merge(libs)?;
    let mut e = Engine::new(m.defaults.clone(), m.cards.iter().map(|(c, t, _)| (c.clone(), t.clone())).collect()).map_err(|e| err("card_rejected", e.to_string()))?;
    let mut notes = Vec::new();
    for (label, r, _) in &m.regs {
        set_regulation_with_local(&mut e, &mut notes, label, r.clone(), local_dir);
    }
    for (c, _, layer) in &m.cards {
        if let Some(x) = &c.xlsx {
            let lib = libs.iter().find(|(n, _)| n == layer).map(|(_, l)| *l).ok_or_else(|| err("missing_file", layer.clone()))?;
            let b = lib.file(&x.template).ok_or_else(|| err("missing_file", format!("xlsx template {} of card {}", x.template, c.id)))?;
            e.attach_xlsx_template(&c.id, b.to_vec()).map_err(|e| err("card_rejected", e.to_string()))?;
        }
    }
    Ok((e, notes, m))
}

impl Stack {
    /// Load the stack of an install: base `library`, `<root>/personal-library` if present,
    /// packs from `<root>/sockets.toml`, where root is the folder holding `library`.
    pub fn load_installed(library: &Path) -> Result<Stack> {
        let root = library.parent().map(Path::to_path_buf).unwrap_or_default();
        let personal = root.join(PERSONAL_DIR);
        let (packs, sockets_err) = match read_sockets(&root) {
            Ok(p) => (p, None),
            Err(e) => (vec![], Some(e)),
        };
        let mut s = Stack::load(library, personal.is_dir().then_some(personal.as_path()), &packs)?;
        if let Some(e) = sockets_err {
            s.layers.insert(0, LayerInfo { name: SOCKETS_FILE.into(), kind: "pack", path: root.join(SOCKETS_FILE), version: None, status: e.to_string(), cards: vec![] });
        }
        Ok(s)
    }

    /// `personal` and `packs` may fail one by one (reported in `layers`); the base may not.
    pub fn load(base: &Path, personal: Option<&Path>, packs: &[PackEntry]) -> Result<Stack> {
        let base_lib = Library::load(&DirSource::new(base))?;
        // candidates, highest priority first
        let mut cand: Vec<(LayerInfo, Result<Library>)> = Vec::new();
        if let Some(p) = personal {
            cand.push((LayerInfo { name: "personal".into(), kind: "personal", path: p.to_path_buf(), version: None, status: String::new(), cards: vec![] }, open_pack(p, false)));
        }
        for pk in packs {
            let lib = open_pack(&pk.path, true).and_then(|l| {
                if l.manifest_sha256 == pk.manifest_sha256 {
                    Ok(l)
                } else {
                    Err(err("pack_changed", format!("manifest.toml이 연결 때와 다르다({}…). 내용을 확인하고 oh library add로 다시 연결한다", &l.manifest_sha256[..12])))
                }
            });
            cand.push((LayerInfo { name: pk.name.clone(), kind: "pack", path: pk.path.clone(), version: None, status: String::new(), cards: vec![] }, lib));
        }
        // accept layers from the base upward; a layer whose cards do not bind is skipped
        let mut accepted: Vec<(String, Library)> = Vec::new();
        let mut infos: Vec<LayerInfo> = Vec::new();
        for (mut info, lib) in cand.into_iter().rev() {
            match lib {
                Err(e) => info.status = e.to_string(),
                Ok(l) => {
                    info.version = Some(l.manifest.version.clone());
                    let mut trial: Vec<(&str, &Library)> = vec![(info.name.as_str(), &l)];
                    trial.extend(accepted.iter().rev().map(|(n, l)| (n.as_str(), l)));
                    trial.push(("base", &base_lib));
                    match build_engine(&trial, None) {
                        Ok(_) => {
                            info.status = "loaded".into();
                            accepted.push((info.name.clone(), l));
                        }
                        Err(e) => info.status = e.to_string(),
                    }
                }
            }
            infos.push(info);
        }
        infos.reverse();
        let mut libs: Vec<(String, Library)> = accepted.into_iter().rev().collect();
        libs.push(("base".into(), base_lib));
        let mut layers = infos;
        layers.push(LayerInfo { name: "base".into(), kind: "base", path: base.to_path_buf(), version: Some(libs.last().unwrap().1.manifest.version.clone()), status: "loaded".into(), cards: vec![] });
        let mut st = Stack { layers, libs, conflicts: vec![], card_layer: BTreeMap::new() };
        let m = merge(&st.views())?;
        for (c, _, layer) in &m.cards {
            st.card_layer.insert(c.id.clone(), layer.clone());
            if let Some(li) = st.layers.iter_mut().find(|x| &x.name == layer) {
                li.cards.push(c.id.clone());
            }
        }
        st.conflicts = m.conflicts;
        Ok(st)
    }

    fn views(&self) -> Vec<(&str, &Library)> {
        self.libs.iter().map(|(n, l)| (n.as_str(), l)).collect()
    }
    pub fn base(&self) -> &Library {
        &self.libs.last().unwrap().1
    }
    /// Loaded libraries, highest priority first.
    pub fn libraries(&self) -> impl Iterator<Item = (&str, &Library)> {
        self.libs.iter().map(|(n, l)| (n.as_str(), l))
    }
    pub fn card_layer(&self, id: &str) -> Option<&str> {
        self.card_layer.get(id).map(String::as_str)
    }
    /// Engine over all loaded layers; company regulation tables from `local_dir` win last.
    pub fn engine_with_local(&self, local_dir: Option<&Path>) -> Result<(Engine, Vec<String>)> {
        build_engine(&self.views(), local_dir).map(|(e, n, _)| (e, n))
    }
    /// Cards in use (after priority), with the layer each comes from.
    pub fn cards(&self) -> Result<Vec<(Card, String)>> {
        Ok(merge(&self.views())?.cards.into_iter().map(|(c, _, l)| (c, l)).collect())
    }
    /// Substring match on id, title and triggers of the cards in use.
    pub fn search(&self, query: &str) -> Result<Vec<StackHit>> {
        let q = query.trim();
        let mut hits: Vec<StackHit> = self
            .cards()?
            .into_iter()
            .filter(|(c, _)| !q.is_empty() && (c.id.contains(q) || c.title.contains(q) || c.triggers.iter().any(|t| t.contains(q) || q.contains(t.as_str()))))
            .map(|(c, l)| {
                let strong = q.contains(c.id.as_str()) || c.triggers.iter().any(|t| q.contains(t.as_str()));
                StackHit { id: c.id, title: c.title, layer: l, fallback: c.fallback, strong }
            })
            .collect();
        hits.sort_by_key(|h| h.fallback); // fallback cards last (stable)
        Ok(hits)
    }
}
