//! `oh doctor`: version, library manifest hashes, work-folder permissions, one synthetic
//! document created in a temporary outbox, and read-only client registration status.
use crate::install::{self, Client};
use oh_library::{DirSource, Library};
use serde_json::{json, Value};
use std::path::Path;

const SPEC: &str = r##"{"page":{"width":59528,"height":84186,"left":8504,"right":8504,"top":5668,"bottom":4252},
"blocks":[{"type":"paragraph","align":"CENTER","runs":[{"text":"oh doctor 합성 문서","height":1600,"bold":true}]},
{"type":"table","column_widths":[18000,24520],"row_heights":[2400,3200],"rows":[[{"text":"항목"},{"text":"결과"}],[{"text":"합성"},{"text":"생성 확인"}]]}]}"##;

fn check(name: &str, r: Result<Value, String>) -> (Value, bool) {
    match r {
        Ok(d) => (json!({"check": name, "ok": true, "detail": d}), true),
        Err(e) => (json!({"check": name, "ok": false, "error": e}), false),
    }
}

fn probe_dir(dir: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("폴더를 만들 수 없다 {}: {e}", dir.display()))?;
    let probe = dir.join(format!(".oh-doctor-probe-{}", std::process::id()));
    oh_host::publish_no_clobber(&probe, b"ok").map_err(|c| format!("{}에 쓸 수 없다: {c}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(json!(dir))
}

fn synthetic() -> Result<Value, String> {
    let spec = oh_document::parse_contract(SPEC.as_bytes(), "invalid_create_spec").map_err(String::from)?;
    let prepared = oh_document::create_bytes(spec).map_err(String::from)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("oh-doctor-{}-{nanos}", std::process::id()));
    let outbox = tmp.join("outbox");
    let run = (|| -> Result<Value, String> {
        std::fs::create_dir_all(&outbox).map_err(|e| format!("임시 outbox 생성 실패: {e}"))?;
        let target = outbox.join("doctor.hwpx");
        oh_host::publish_no_clobber(&target, &prepared.bytes).map_err(|c| format!("저장 실패: {c}"))?;
        let back = oh_host::bounded_read(&target, oh_document::MAX_INPUT).map_err(|c| format!("읽기 실패: {c}"))?;
        let pkg = oh_document::Package::from_bytes(back.clone()).map_err(String::from)?;
        let texts: Vec<&str> = pkg.inspection.text_nodes.iter().map(|n| n.text.as_str()).collect();
        if !texts.contains(&"생성 확인") {
            return Err("생성한 문서에서 기대한 텍스트를 찾지 못했다".into());
        }
        Ok(json!({"bytes": back.len(), "sha256": oh_recipe::hex_sha256(&back), "text_nodes": texts.len()}))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    run
}

pub fn run(library: &Path, workdir: &Path) -> (Value, bool) {
    let mut checks = Vec::new();
    let mut all = true;
    let mut add = |(v, ok): (Value, bool)| {
        all &= ok;
        checks.push(v);
    };
    add(check("version", Ok(json!({"oh": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "exe": std::env::current_exe().ok()}))));
    add(check("library", Library::load(&DirSource::new(library)).map_err(|e| e.to_string()).and_then(|lib| {
        let eng = lib.engine().map_err(|e| e.to_string())?;
        Ok(json!({"path": library, "version": lib.manifest.version, "anonymized": lib.manifest.anonymized, "files_hash_verified": lib.manifest.files.len(),
            "search": lib.manifest.search, "cards": eng.card_ids()}))
    })));
    // socket layers (D12): every connected pack and the personal library must load
    add(match oh_library::Stack::load_installed(library) {
        Ok(s) => {
            let ok = s.layers.iter().all(|l| l.status == "loaded");
            (json!({"check": "library_layers", "ok": ok, "detail": {
                "layers": s.layers.iter().map(|l| json!({"name": l.name, "kind": l.kind, "path": l.path, "version": l.version, "status": l.status, "cards": l.cards})).collect::<Vec<_>>(),
                "conflicts": s.conflicts.iter().map(|c| json!({"kind": c.kind, "id": c.id, "used": c.used, "shadowed": c.shadowed})).collect::<Vec<_>>()}}), ok)
        }
        Err(e) => (json!({"check": "library_layers", "ok": false, "detail": e.to_string()}), false),
    });
    add(check("workdir", (|| {
        let mut v = serde_json::Map::new();
        for s in ["inbox", "work", "outbox"] {
            v.insert(s.into(), probe_dir(&workdir.join(s))?);
        }
        Ok(Value::Object(v))
    })()));
    add(check("synthetic_document", synthetic()));
    let clients: serde_json::Map<String, Value> = Client::ALL.iter().map(|(n, c)| (n.to_string(), install::registered(*c))).collect();
    checks.push(json!({"check": "client_registration", "ok": true, "info_only": true, "detail": clients}));
    (json!({"ok": all, "checks": checks}), all)
}
