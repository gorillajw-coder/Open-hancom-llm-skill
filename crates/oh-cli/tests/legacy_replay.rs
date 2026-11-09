//! Replay of every distinct `open-hancom-core` invocation made by the legacy
//! Python test-suite (P0 baseline), against the new `oh` binary. Expected exit
//! code, stdout (minus `path`) and output sha256 were produced by the LEGACY P0
//! binary (scripts/parity-wrapper.py CAPTURE=1 → scripts/build-corpus.py).
//! Portable: the corpus is embedded and every path is created under a temp dir.
mod common;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Command;

const CORPUS: &[u8] = include_bytes!("corpus/legacy-calls.zip");

struct Corpus { cases: Vec<Value>, blobs: HashMap<String, Vec<u8>> }

fn corpus() -> &'static Corpus {
    static C: std::sync::OnceLock<Corpus> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let mut z = zip::ZipArchive::new(Cursor::new(CORPUS)).unwrap();
        let mut cases = Vec::new();
        let mut blobs = HashMap::new();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            let name = f.name().to_string();
            if name == "cases.jsonl" {
                cases = String::from_utf8(data).unwrap().lines().filter(|l| !l.is_empty())
                    .map(|l| serde_json::from_str(l).unwrap()).collect();
            } else if let Some(h) = name.strip_prefix("blobs/") {
                blobs.insert(h.to_string(), data);
            }
        }
        Corpus { cases, blobs }
    })
}

/// An argument that is not valid Unicode: raw 0xFF on Unix, an unpaired
/// UTF-16 surrogate on Windows (the only failure mode of UTF-16 → UTF-8).
fn non_utf8_arg() -> std::ffi::OsString {
    #[cfg(unix)] { use std::os::unix::ffi::OsStringExt; std::ffi::OsString::from_vec(vec![b'a', 0xFF]) }
    #[cfg(windows)] { use std::os::windows::ffi::OsStringExt; std::ffi::OsString::from_wide(&[0x61, 0xD800]) }
}

fn sha(b: &[u8]) -> String { format!("{:x}", Sha256::digest(b)) }

fn strip_path(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() { o.remove("path"); }
    v
}

/// Run one case; returns a description of the mismatch, if any.
fn replay(case: &Value, blobs: &HashMap<String, Vec<u8>>) -> Option<String> {
    let dir = common::tempdir();
    let d = dir.path();
    let mut args = Vec::new();
    let mut output: Option<(std::path::PathBuf, Option<Vec<u8>>)> = None;
    for (i, a) in case["argv"].as_array().unwrap().iter().enumerate() {
        if let Some(l) = a["lit"].as_str() { args.push(l.into()); continue; }
        if let Some(h) = a["file"].as_str() {
            let p = d.join(format!("in{i}.bin"));
            std::fs::write(&p, &blobs[h]).unwrap();
            args.push(p.into_os_string()); continue;
        }
        if a["dir"].as_bool() == Some(true) {
            let p = d.join(format!("dir{i}"));
            std::fs::create_dir(&p).unwrap();
            args.push(p.into_os_string()); continue;
        }
        if a["non_utf8"].as_bool() == Some(true) { args.push(non_utf8_arg()); continue; }
        if a["missing"].as_bool() == Some(true) { args.push(d.join(format!("missing{i}")).into_os_string()); continue; }
        match a["out"].as_str() {
            Some("new") => { let p = d.join(format!("out{i}.hwpx")); output = Some((p.clone(), None)); args.push(p.into_os_string()); }
            Some("no_parent") => args.push(d.join("no-such-dir").join("out.hwpx").into_os_string()),
            Some("exists") => {
                let p = d.join(format!("out{i}.hwpx"));
                let before = match a["blob"].as_str() {
                    Some(h) => { std::fs::write(&p, &blobs[h]).unwrap(); Some(blobs[h].clone()) }
                    None => { std::fs::create_dir(&p).unwrap(); None }
                };
                output = Some((p.clone(), before));
                args.push(p.into_os_string());
            }
            _ => panic!("unknown argv entry {a}"),
        }
    }
    let run = Command::new(env!("CARGO_BIN_EXE_oh")).args(&args).output().unwrap();
    let rc = run.status.code();
    let label = || format!("{}", case["argv"]);
    if rc != case["rc"].as_i64().map(|v| v as i32) {
        return Some(format!("{}: rc {rc:?} != {}", label(), case["rc"]));
    }
    let got: Value = match serde_json::from_slice(&run.stdout) {
        Ok(v) => strip_path(v),
        Err(_) => Value::String(String::from_utf8_lossy(&run.stdout).into()),
    };
    if got != case["stdout"] {
        return Some(format!("{}: stdout {} != {}", label(), got, case["stdout"]));
    }
    if let Some((p, before)) = output {
        match (rc, case["output_sha256"].as_str(), before) {
            (Some(0), Some(want), None) => {
                let have = sha(&std::fs::read(&p).unwrap());
                if have != want { return Some(format!("{}: output sha {have} != {want}", label())); }
            }
            (_, _, Some(before)) => {
                if std::fs::read(&p).ok().as_deref() != Some(before.as_slice()) {
                    return Some(format!("{}: existing target modified", label()));
                }
            }
            (Some(0), None, None) => {}
            (_, _, None) => {
                if p.symlink_metadata().is_ok() && !Path::new(&p).is_dir() {
                    return Some(format!("{}: failed call left an output file", label()));
                }
            }
        }
    }
    // no stray stage files may remain
    let stray: Vec<_> = std::fs::read_dir(d).unwrap().filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".oh-stage")).collect();
    if !stray.is_empty() { return Some(format!("{}: stage files left {stray:?}", label())); }
    None
}

fn run_group(cmds: &[&str]) {
    let c = corpus();
    let cases: Vec<&Value> = c.cases.iter().filter(|v| cmds.contains(&v["argv"][0]["lit"].as_str().unwrap_or(""))).collect();
    assert!(!cases.is_empty(), "no cases for {cmds:?}");
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(case) = cases.get(i) else { break };
                if let Some(f) = replay(case, &c.blobs) { failures.lock().unwrap().push(f); }
            });
        }
    });
    let f = failures.into_inner().unwrap();
    assert!(f.is_empty(), "{} of {} cases differ from legacy:\n{}", f.len(), cases.len(), f[..f.len().min(20)].join("\n"));
    eprintln!("{cmds:?}: {} legacy cases identical", cases.len());
}

// One test per P1 vertical slice.
#[test] fn slice1_read_only_matches_legacy() { run_group(&["schema", "inspect", "validate", "diff"]) }
#[test] fn slice2_3_apply_and_publish_match_legacy() { run_group(&["apply"]) }
#[test] fn slice4_create_matches_legacy() { run_group(&["create"]) }
#[test] fn slice5_edit_commands_match_legacy() {
    run_group(&["format", "paragraph", "table", "style", "border", "image", "link", "image-insert", "image-replace"])
}
#[test] fn corpus_covers_every_legacy_command() {
    let c = corpus();
    for cmd in ["schema", "inspect", "validate", "diff", "create", "apply", "format", "paragraph", "style", "border", "table", "image", "link", "image-insert", "image-replace"] {
        for rc in [0, 2] {
            let n = c.cases.iter().filter(|v| v["argv"][0]["lit"] == cmd && v["rc"] == rc).count();
            if !(cmd == "schema" || cmd == "validate" || cmd == "diff") || rc == 0 { assert!(n > 0, "{cmd} rc={rc} has no case"); }
        }
    }
}
