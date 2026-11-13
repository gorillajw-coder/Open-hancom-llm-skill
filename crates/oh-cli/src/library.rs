//! `oh library list|add|remove`: connect shared library packs to the socket (D12).
//! Connections are written to `<install root>/sockets.toml`; nothing is copied or deleted.
use oh_library::{authoring, open_pack, read_sockets, valid_pack_name, write_sockets, PackEntry, Stack, PERSONAL_DIR};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub type Fail = (&'static str, String);

fn layers(s: &Stack) -> Value {
    json!({
        "order": "personal > packs > base",
        "layers": s.layers.iter().map(|l| json!({"name": l.name, "kind": l.kind, "path": l.path, "version": l.version, "status": l.status, "cards": l.cards})).collect::<Vec<_>>(),
        "conflicts": s.conflicts.iter().map(|c| json!({"kind": c.kind, "id": c.id, "used": c.used, "shadowed": c.shadowed})).collect::<Vec<_>>(),
    })
}
fn stack(root: &Path, packs: &[PackEntry]) -> Result<Stack, Fail> {
    let personal = root.join(PERSONAL_DIR);
    Stack::load(&root.join("library"), personal.is_dir().then_some(personal.as_path()), packs).map_err(|e| ("library_invalid", e.to_string()))
}

pub fn list(root: &Path) -> Result<Value, Fail> {
    let packs = read_sockets(root).map_err(|e| (e.code, e.detail))?;
    let s = stack(root, &packs)?;
    Ok(json!({"root": root, "library": layers(&s)}))
}

struct Trial {
    lib: oh_library::Library,
    name: String,
    path: PathBuf,
    packs: Vec<PackEntry>,
    replaced: bool,
    stack: Stack,
}

/// Load the pack into the full stack as `oh library add` would, without recording anything.
fn trial(root: &Path, path: &Path, name: Option<&str>) -> Result<Trial, Fail> {
    let path: PathBuf = std::path::absolute(path).map_err(|e| ("invalid_arguments", e.to_string()))?;
    let lib = open_pack(&path, true).map_err(|e| (e.code, e.detail))?;
    let name = match name {
        Some(n) => n.to_string(),
        None => lib
            .manifest
            .name
            .clone()
            .or_else(|| path.file_stem().and_then(|s| s.to_str()).map(|s| s.split_whitespace().collect::<Vec<_>>().join("-")))
            .unwrap_or_default(),
    };
    if !valid_pack_name(&name) {
        return Err(("invalid_arguments", format!("팩 이름은 1~40자 글자·숫자·-_. 만(base/personal 제외): {name:?}. --name으로 정한다")));
    }
    let mut packs = read_sockets(root).map_err(|e| (e.code, e.detail))?;
    let before = stack(root, &packs)?;
    // the same pack under another spelling of its path (`..`, case on Windows) is the same pack
    let canon = |p: &Path| p.canonicalize().ok();
    if let Some(other) = packs.iter().find(|p| p.name != name && (p.path == path || (canon(&p.path).is_some() && canon(&p.path) == canon(&path)))) {
        return Err(("pack_already_connected", format!("같은 경로가 이미 '{}'로 연결돼 있다", other.name)));
    }
    let entry = PackEntry { name: name.clone(), path: path.clone(), manifest_sha256: lib.manifest_sha256.clone(), added: oh_mcp::now_stamp() };
    let replaced = match packs.iter_mut().find(|p| p.name == name) {
        Some(p) => {
            *p = entry;
            true
        }
        None => {
            packs.push(entry);
            false
        }
    };
    // the pack must load in the full stack (its cards bind, frames resolve) before it is recorded
    let s = stack(root, &packs)?;
    let st = s.layers.iter().find(|l| l.name == name).map(|l| l.status.clone()).unwrap_or_default();
    if st != "loaded" {
        return Err(("pack_rejected", format!("{name}: {st}. 연결하지 않았다")));
    }
    // a new pack may not knock out a layer that loaded before (e.g. a personal card whose frame it changes)
    let lost: Vec<String> = before
        .layers
        .iter()
        .filter(|l| l.status == "loaded" && l.name != name)
        .filter(|l| s.layers.iter().find(|x| x.name == l.name).is_none_or(|x| x.status != "loaded"))
        .map(|l| format!("{}: {}", l.name, s.layers.iter().find(|x| x.name == l.name).map(|x| x.status.as_str()).unwrap_or("빠짐")))
        .collect();
    if !lost.is_empty() {
        return Err(("pack_rejected", format!("{name}을 연결하면 다른 층이 빠진다({}). 연결하지 않았다", lost.join("; "))));
    }
    Ok(Trial { lib, name, path, packs, replaced, stack: s })
}

pub fn add(root: &Path, path: &Path, name: Option<&str>) -> Result<Value, Fail> {
    let t = trial(root, path, name)?;
    write_sockets(root, &t.packs).map_err(|e| (e.code, e.detail))?;
    Ok(json!({"connected": t.name, "replaced": t.replaced, "path": t.path, "version": t.lib.manifest.version, "manifest_sha256": t.lib.manifest_sha256, "library": layers(&t.stack)}))
}

pub fn remove(root: &Path, name: &str) -> Result<Value, Fail> {
    let mut packs = read_sockets(root).map_err(|e| (e.code, e.detail))?;
    let before = packs.len();
    packs.retain(|p| p.name != name);
    if packs.len() == before {
        return Err(("pack_not_connected", name.to_string()));
    }
    write_sockets(root, &packs).map_err(|e| (e.code, e.detail))?;
    let s = stack(root, &packs)?;
    // a layer that relied on the removed pack (its frame, say) is reported, not hidden
    let warnings: Vec<String> = s.layers.iter().filter(|l| l.status != "loaded").map(|l| format!("{}: {}", l.name, l.status)).collect();
    Ok(json!({"removed": name, "files_deleted": false, "warnings": warnings, "library": layers(&s)}))
}

// ---------------------------------------------------------------- pack authoring (1.1.0)

/// Largest single file the scan reads (a pack zip may be this big, LIBRARY-GUIDE 2).
const SCAN_FILE_MAX: u64 = 256 * 1024 * 1024;

pub fn read_terms(path: &Path) -> Result<Vec<(usize, String)>, Fail> {
    let b = oh_host::bounded_read(path, 4 * 1024 * 1024).map_err(|c| (c, format!("검사어 파일 {}", path.display())))?;
    let t = String::from_utf8(b).map_err(|_| ("invalid_terms", "검사어 파일은 UTF-8이어야 한다".to_string()))?;
    let terms = authoring::parse_terms(&t);
    if terms.is_empty() {
        return Err(("invalid_terms", "검사어가 하나도 없다(한 줄에 하나)".into()));
    }
    Ok(terms)
}

/// Every file under `path` (or `path` itself), relative names with `/`.
fn walk(path: &Path) -> Result<Vec<(String, PathBuf)>, Fail> {
    if !path.is_dir() {
        let shown = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        return Ok(vec![(shown, path.to_path_buf())]);
    }
    let mut out = Vec::new();
    let mut todo = vec![(String::new(), path.to_path_buf())];
    while let Some((rel, dir)) = todo.pop() {
        for (n, is_dir) in oh_host::list_dir(&dir).map_err(|c| (c, dir.display().to_string()))? {
            let r = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
            if is_dir {
                todo.push((r, dir.join(&n)));
            } else {
                out.push((r, dir.join(&n)));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn scan_files(files: &[(String, PathBuf)], terms: &[(usize, String)]) -> Result<authoring::ScanReport, Fail> {
    let mut r = authoring::ScanReport::default();
    for (rel, p) in files {
        authoring::scan_name(rel, rel, terms, &mut r);
        match oh_host::bounded_read(p, SCAN_FILE_MAX) {
            Ok(b) => authoring::scan_bytes(rel, &b, terms, &mut r),
            Err(c) => r.unreadable.push(format!("{rel} ({c})")),
        }
    }
    Ok(r)
}

/// `oh library scan --path P --terms T`: residual scan of a file, folder or pack zip.
pub fn scan(path: &Path, terms: &Path) -> Result<(Value, bool), Fail> {
    let terms = read_terms(terms)?;
    let files = walk(path)?;
    let r = scan_files(&files, &terms)?;
    let clean = r.clean();
    Ok((json!({"clean": clean, "path": path, "terms": terms.len(), "report": r, "note": "검사어는 줄 번호로만 표시한다"}), clean))
}

/// `[[replace]] from = "…" to = "…"` (applied in file order).
fn read_map(path: &Path) -> Result<Vec<(String, String)>, Fail> {
    let b = oh_host::bounded_read(path, 4 * 1024 * 1024).map_err(|c| (c, format!("치환표 {}", path.display())))?;
    let t = String::from_utf8(b).map_err(|_| ("invalid_anonymize_map", "치환표는 UTF-8 TOML이어야 한다".to_string()))?;
    let doc: toml_edit::DocumentMut = t.parse().map_err(|e: toml_edit::TomlError| ("invalid_anonymize_map", e.to_string()))?;
    let arr = doc.get("replace").and_then(|v| v.as_array_of_tables()).ok_or(("invalid_anonymize_map", "[[replace]] 표가 없다".to_string()))?;
    let mut out = Vec::new();
    for (i, tb) in arr.iter().enumerate() {
        let get = |k: &str| tb.get(k).and_then(|v| v.as_str()).map(str::to_string);
        match (get("from"), get("to")) {
            (Some(a), Some(b)) if !a.is_empty() => out.push((a, b)),
            _ => return Err(("invalid_anonymize_map", format!("{}번째 [[replace]]: from(빈 값 불가)·to 문자열 필요", i + 1))),
        }
        if let Some(k) = tb.iter().map(|(k, _)| k).find(|k| *k != "from" && *k != "to") {
            return Err(("invalid_anonymize_map", format!("{}번째 [[replace]]: 모르는 키 {k}", i + 1)));
        }
    }
    if out.is_empty() {
        return Err(("invalid_anonymize_map", "치환 항목이 없다".into()));
    }
    Ok(out)
}

/// `oh library anonymize --path IN.hwpx --map M.toml --output OUT.hwpx [--terms T]`.
/// Never overwrites: OUT must not exist. With `--terms` the result is scanned at once.
pub fn anonymize(input: &Path, map: &Path, output: &Path, terms: Option<&Path>) -> Result<(Value, bool), Fail> {
    let pairs = read_map(map)?;
    let terms = terms.map(read_terms).transpose()?;
    let raw = oh_host::bounded_read(input, oh_document::MAX_INPUT).map_err(|c| (c, input.display().to_string()))?;
    let pkg = oh_document::Package::from_bytes(raw).map_err(|c| (c, "HWPX만 받는다".to_string()))?;
    let p = pkg.anonymize(&pairs).map_err(|c| (c, String::new()))?;
    oh_host::publish_no_clobber(output, &p.bytes).map_err(|c| (c, output.display().to_string()))?;
    let mut v = p.receipt;
    v["path"] = json!(output);
    let mut ok = v["remaining"].as_array().is_some_and(|a| a.is_empty());
    if let Some(t) = terms {
        let name = output.file_name().and_then(|n| n.to_str()).unwrap_or("output").to_string();
        let r = scan_files(&[(name, output.to_path_buf())], &t)?;
        ok &= r.clean();
        v["scan"] = json!({"clean": r.clean(), "report": r});
    }
    v["clean"] = json!(ok);
    Ok((v, ok))
}

/// `oh library manifest --path DIR [--version V] [--name N] [--terms T]`: rewrite `manifest.toml`
/// with the sha256 of every file in the pack folders. `anonymized = true` only when `--terms` was
/// given and the scan of every listed file is clean (LIBRARY-GUIDE 4). The pack is then loaded
/// once with full hash and card checks.
pub fn manifest(dir: &Path, version: Option<&str>, name: Option<&str>, terms: Option<&Path>) -> Result<(Value, bool), Fail> {
    if !dir.is_dir() {
        return Err(("invalid_arguments", format!("--path는 팩 폴더여야 한다: {}", dir.display())));
    }
    let terms = terms.map(read_terms).transpose()?;
    // what the old manifest said (kept unless a flag replaces it)
    let mpath = dir.join("manifest.toml");
    let (mut old_version, mut old_name, mut old_search) = (None, None, None);
    if mpath.is_file() {
        let b = oh_host::bounded_read(&mpath, oh_library::MAX_FILE).map_err(|c| (c, "manifest.toml".to_string()))?;
        let doc: toml_edit::DocumentMut = String::from_utf8_lossy(&b).parse().map_err(|e: toml_edit::TomlError| ("invalid_manifest", e.to_string()))?;
        let s = |k: &str| doc.get(k).and_then(|v| v.as_str()).map(str::to_string);
        (old_version, old_name, old_search) = (s("version"), s("name"), s("search"));
    }
    let version = version.map(str::to_string).or(old_version).ok_or(("invalid_arguments", "--version 필요(기존 manifest에 버전이 없다)".to_string()))?;
    let name = name.map(str::to_string).or(old_name);
    if let Some(n) = name.as_ref().filter(|n| !valid_pack_name(n)) {
        return Err(("invalid_arguments", format!("팩 이름은 1~40자 글자·숫자·-_. 만(base/personal 제외): {n:?}")));
    }
    let search = old_search.unwrap_or_else(|| "none: substring only in P3; TF-IDF index is P5 (not neural)".to_string());
    let mut rows = Vec::new();
    let mut files = Vec::new();
    let mut not_listed = Vec::new();
    // order as scripts/library-manifest.py: the pack folders in PACK_DIRS order, defaults.toml last
    let top = oh_host::list_dir(dir).map_err(|c| (c, dir.display().to_string()))?;
    for (d, _) in authoring::PACK_DIRS {
        if !top.iter().any(|(n, is_dir)| n == d && *is_dir) {
            continue;
        }
        for (f, sub) in oh_host::list_dir(&dir.join(d)).map_err(|c| (c, d.to_string()))? {
            let rel = format!("{d}/{f}");
            match (sub, authoring::kind_for(d, &f)) {
                (false, Some(k)) => files.push((rel, dir.join(d).join(&f), k)),
                _ => not_listed.push(if sub { format!("{rel}/") } else { rel }),
            }
        }
    }
    for (n, is_dir) in &top {
        if n == "manifest.toml" || (*is_dir && authoring::PACK_DIRS.iter().any(|(d, _)| d == n)) {
            continue;
        }
        if n == "defaults.toml" && !is_dir {
            files.push((n.clone(), dir.join(n), "defaults"));
        } else {
            not_listed.push(if *is_dir { format!("{n}/") } else { n.clone() });
        }
    }
    if files.is_empty() {
        return Err(("invalid_pack", "팩 폴더(cards·templates·frames·examples·regulations, defaults.toml)에 파일이 없다".into()));
    }
    for (rel, p, k) in &files {
        let b = oh_host::bounded_read(p, oh_library::MAX_FILE).map_err(|c| (c, rel.clone()))?;
        rows.push((rel.clone(), oh_recipe::hex_sha256(&b), k.to_string()));
    }
    let scan = match &terms {
        Some(t) => Some(scan_files(&files.iter().map(|(r, p, _)| (r.clone(), p.clone())).collect::<Vec<_>>(), t)?),
        None => None,
    };
    let anonymized = scan.as_ref().is_some_and(|r| r.clean());
    let text = authoring::manifest_text(&version, anonymized, name.as_deref(), &search, &rows);
    oh_host::replace_file(&mpath, text.as_bytes()).map_err(|c| (c, mpath.display().to_string()))?;
    let load = match open_pack(dir, anonymized) {
        Ok(lib) => json!({"ok": true, "cards": lib.manifest.files.iter().filter(|f| f.kind == "card").count(), "manifest_sha256": lib.manifest_sha256}),
        Err(e) => json!({"ok": false, "error": e.code, "detail": e.detail}),
    };
    let ok = load["ok"] == true && (terms.is_none() || anonymized);
    let note = match (&terms, anonymized) {
        (None, _) => "검사어(--terms)가 없어 anonymized = false로 썼다. 개인 라이브러리용이다. 공용 팩은 --terms로 다시 실행한다",
        (Some(_), false) => "잔존 스캔에 걸려 anonymized = false로 썼다. 공용 팩으로 연결할 수 없다. scan 결과를 고친 뒤 다시 실행한다",
        (Some(_), true) => "잔존 스캔 0, anonymized = true",
    };
    Ok((json!({"ok": ok, "path": mpath, "version": version, "name": name, "anonymized": anonymized, "files": rows.len(), "not_listed": not_listed, "load": load, "scan": scan, "note": note}), ok))
}

/// `oh library check --path P [--terms T] [--name N] [--root R]`: everything `add` checks
/// (hashes, anonymized flag, cards bind in the installed stack, no layer knocked out), plus the
/// residual scan with `--terms`; nothing is recorded.
pub fn check(root: &Path, path: &Path, name: Option<&str>, terms: Option<&Path>) -> Result<(Value, bool), Fail> {
    let terms = terms.map(read_terms).transpose()?;
    let mut v = match trial(root, path, name) {
        Ok(t) => json!({"loads": true, "name": t.name, "would_replace": t.replaced, "version": t.lib.manifest.version, "manifest_sha256": t.lib.manifest_sha256, "library": layers(&t.stack)}),
        Err((c, d)) => json!({"loads": false, "error": c, "detail": d}),
    };
    let mut ok = v["loads"] == true;
    if let Some(t) = terms {
        let r = scan_files(&walk(path)?, &t)?;
        ok &= r.clean();
        v["scan"] = json!({"clean": r.clean(), "report": r});
    }
    v["ok"] = json!(ok);
    v["recorded"] = json!(false);
    Ok((v, ok))
}
