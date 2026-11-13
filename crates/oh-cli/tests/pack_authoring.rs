//! Pack authoring commands (1.1.0): `oh library manifest|scan|anonymize|check`. Real `oh`
//! processes on copies of the shipped library; the "private" term is the placeholder name that
//! the shipped templates carry, so no real term list is needed.
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
mod common;

const TERM: &str = "김철수";

fn repo_lib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}
fn copy_dir(a: &Path, b: &Path) {
    std::fs::create_dir_all(b).unwrap();
    for e in std::fs::read_dir(a).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &b.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), b.join(e.file_name())).unwrap();
        }
    }
}
fn oh(args: &[&str]) -> (Value, i32) {
    let o = Command::new(env!("CARGO_BIN_EXE_oh")).args(args).output().unwrap();
    let out = String::from_utf8(o.stdout).unwrap();
    (serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}")), o.status.code().unwrap())
}
fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}
fn terms(d: &Path, lines: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let p = d.join(format!("검사어 {}.txt", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::write(&p, lines).unwrap();
    p
}
fn map(d: &Path, from: &str, to: &str) -> PathBuf {
    let p = d.join(format!("치환표-{}.toml", to.len()));
    std::fs::write(&p, format!("[[replace]]\nfrom = \"{from}\"\nto = \"{to}\"\n")).unwrap();
    p
}
fn findings(v: &Value) -> Vec<Value> {
    v["report"]["findings"].as_array().cloned().unwrap_or_default()
}

#[test]
fn manifest_reproduces_the_shipped_manifest_byte_for_byte() {
    let d = common::tempdir();
    let lib = d.path().join("라이브러리 사본");
    copy_dir(&repo_lib(), &lib);
    let shipped = std::fs::read(repo_lib().join("manifest.toml")).unwrap();
    let version = String::from_utf8_lossy(&shipped).lines().next().unwrap().trim_start_matches("version = ").trim_matches('"').to_string();
    let t = terms(d.path(), "없는회사이름\nQzxvw\n");
    // existing version and search line are kept, hashes and order are recomputed
    let (v, code) = oh(&["library", "manifest", "--path", s(&lib), "--terms", s(&t)]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["anonymized"], true);
    assert_eq!(v["load"]["ok"], true);
    assert_eq!(std::fs::read(lib.join("manifest.toml")).unwrap(), shipped);
    // a fresh folder (no manifest) takes the version from the flag
    std::fs::remove_file(lib.join("manifest.toml")).unwrap();
    let (v, code) = oh(&["library", "manifest", "--path", s(&lib), "--version", &version, "--terms", s(&t)]);
    assert_eq!((code, v["anonymized"].clone()), (0, Value::Bool(true)), "{v}");
    assert!(String::from_utf8(std::fs::read(lib.join("manifest.toml")).unwrap()).unwrap().starts_with(&format!("version = \"{version}\"\nanonymized = true\n")));
    // without terms the same folder is written as not anonymized (personal use), and says so
    let (v, code) = oh(&["library", "manifest", "--path", s(&lib)]);
    assert_eq!((code, v["anonymized"].clone()), (0, Value::Bool(false)), "{v}");
    assert!(String::from_utf8(std::fs::read(lib.join("manifest.toml")).unwrap()).unwrap().contains(&format!("version = \"{version}\"\nanonymized = false\n")));
    // a stray file is reported, not listed
    std::fs::write(lib.join("cards/notes.txt"), "x").unwrap();
    let (v, _) = oh(&["library", "manifest", "--path", s(&lib), "--terms", s(&t)]);
    assert!(v["not_listed"].as_array().unwrap().iter().any(|x| x == "cards/notes.txt"), "{v}");
}

#[test]
fn scan_finds_terms_and_patterns_by_line_number_only() {
    let d = common::tempdir();
    let t = terms(d.path(), &format!("\n{TERM}\nx\n"));
    let tpl = repo_lib().join("templates/official-letter.hwpx");
    let (v, code) = oh(&["library", "scan", "--path", s(&tpl), "--terms", s(&t)]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["clean"], false);
    let f = findings(&v);
    assert!(f.iter().any(|h| h["path"] == "official-letter.hwpx!Contents/section0.xml" && h["terms"] == serde_json::json!([2])), "{v}");
    assert!(!v.to_string().contains(TERM), "the report must not repeat the term");
    // the shipped library is clean for a term it does not hold
    let t2 = terms(d.path(), "없는회사이름\n");
    let (v, code) = oh(&["library", "scan", "--path", s(&repo_lib()), "--terms", s(&t2)]);
    assert_eq!((code, v["clean"].clone()), (0, Value::Bool(true)), "{v}");
}

#[test]
fn anonymize_replaces_rescans_and_never_overwrites() {
    let d = common::tempdir();
    let t = terms(d.path(), &format!("{TERM}\n"));
    let tpl = repo_lib().join("templates/official-letter.hwpx");
    let out = d.path().join("익명 결과.hwpx");
    let m = map(d.path(), TERM, "홍길동");
    let (v, code) = oh(&["library", "anonymize", "--path", s(&tpl), "--map", s(&m), "--output", s(&out), "--terms", s(&t)]);
    assert_eq!(code, 0, "{v}");
    assert!(v["replaced"][0]["count"].as_u64().unwrap() >= 1, "{v}");
    assert_eq!(v["remaining"], serde_json::json!([]));
    assert_eq!(v["scan"]["clean"], true, "{v}");
    let (val, _) = oh(&["validate", s(&out)]);
    assert!(val["diagnostics"].is_array(), "the result reopens: {val}");
    // never overwrites
    let before = std::fs::read(&out).unwrap();
    let (v, code) = oh(&["library", "anonymize", "--path", s(&tpl), "--map", s(&m), "--output", s(&out)]);
    assert_eq!((code, v["error"].clone()), (2, Value::from("target_exists")), "{v}");
    assert_eq!(std::fs::read(&out).unwrap(), before);
    // an e-mail address put in by the map is caught by the scan of the result
    let out2 = d.path().join("메일.hwpx");
    let mail = format!("lee{}corp.co.kr", '@');
    let m2 = map(d.path(), TERM, &mail);
    let (v, code) = oh(&["library", "anonymize", "--path", s(&tpl), "--map", s(&m2), "--output", s(&out2), "--terms", s(&t)]);
    assert_eq!(code, 2, "{v}");
    assert!(v["scan"]["report"]["findings"].as_array().unwrap().iter().any(|h| h["email"].as_u64().unwrap() > 0), "{v}");
}

#[test]
fn check_runs_the_add_checks_without_recording() {
    let d = common::tempdir();
    let root = d.path().join("설치 폴더");
    copy_dir(&repo_lib(), &root.join("library"));
    let pack = d.path().join("팀 팩");
    std::fs::create_dir_all(pack.join("cards")).unwrap();
    std::fs::create_dir_all(pack.join("templates")).unwrap();
    std::fs::copy(repo_lib().join("templates/official-letter.hwpx"), pack.join("templates/official-letter.hwpx")).unwrap();
    std::fs::copy(repo_lib().join("cards/client-report-letter.toml"), pack.join("cards/client-report-letter.toml")).unwrap();
    let t = terms(d.path(), "없는회사이름\n");
    let (v, code) = oh(&["library", "manifest", "--path", s(&pack), "--version", "1.0.0", "--name", "team1", "--terms", s(&t)]);
    assert_eq!((code, v["anonymized"].clone()), (0, Value::Bool(true)), "{v}");
    let (v, code) = oh(&["library", "check", "--path", s(&pack), "--terms", s(&t), "--root", s(&root)]);
    assert_eq!(code, 0, "{v}");
    assert_eq!((v["loads"].clone(), v["name"].clone(), v["recorded"].clone()), (Value::Bool(true), Value::from("team1"), Value::Bool(false)));
    assert!(!root.join("sockets.toml").exists(), "check must not record a connection");
    // a term in the pack: manifest refuses anonymized = true, check fails
    let hit = terms(d.path(), "client-report-letter\n");
    let (v, code) = oh(&["library", "manifest", "--path", s(&pack), "--terms", s(&hit)]);
    assert_eq!((code, v["anonymized"].clone()), (2, Value::Bool(false)), "{v}");
    let (v, code) = oh(&["library", "check", "--path", s(&pack), "--root", s(&root)]);
    assert_eq!((code, v["error"].clone()), (2, Value::from("library_not_anonymized")), "{v}");
    // a file changed after the manifest
    oh(&["library", "manifest", "--path", s(&pack), "--terms", s(&t)]);
    std::fs::write(pack.join("cards/client-report-letter.toml"), std::fs::read_to_string(pack.join("cards/client-report-letter.toml")).unwrap() + "\n# x\n").unwrap();
    let (v, code) = oh(&["library", "check", "--path", s(&pack), "--root", s(&root)]);
    assert_eq!((code, v["error"].clone()), (2, Value::from("manifest_hash_mismatch")), "{v}");
}
