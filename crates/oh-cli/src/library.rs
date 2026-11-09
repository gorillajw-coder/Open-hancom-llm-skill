//! `oh library list|add|remove`: connect shared library packs to the socket (D12).
//! Connections are written to `<install root>/sockets.toml`; nothing is copied or deleted.
use oh_library::{open_pack, read_sockets, valid_pack_name, write_sockets, PackEntry, Stack, PERSONAL_DIR};
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

pub fn add(root: &Path, path: &Path, name: Option<&str>) -> Result<Value, Fail> {
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
    write_sockets(root, &packs).map_err(|e| (e.code, e.detail))?;
    Ok(json!({"connected": name, "replaced": replaced, "path": path, "version": lib.manifest.version, "manifest_sha256": lib.manifest_sha256, "library": layers(&s)}))
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
