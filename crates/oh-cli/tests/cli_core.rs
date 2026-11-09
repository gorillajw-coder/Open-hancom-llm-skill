//! Hand-written CLI integration tests for the core success / rejection /
//! preservation cases of the legacy Python suite (test_rust_core.py,
//! test_rust_create.py, test_safety.py). Expected values were checked against the
//! legacy P0 binary. Portable: fixtures are embedded, paths live in a temp dir.
mod common;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;

const SYNTH: &[u8] = include_bytes!("../../oh-document/tests/fixtures/synthetic.hwpx");
const APPLY: &str = include_str!("../../oh-document/tests/fixtures/apply.json");
const CREATE: &str = include_str!("../../oh-document/tests/fixtures/create.json");

fn oh(args: &[&dyn AsRef<std::ffi::OsStr>]) -> (i32, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_oh")).args(args.iter().map(|a| a.as_ref())).output().unwrap();
    assert!(out.stderr.is_empty(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap(), serde_json::from_slice(&out.stdout).unwrap())
}
fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<_> = fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    v.sort();
    v
}
fn failed(code: &str) -> Value { json!({"error": code, "written": false}) }

#[test]
fn errors_are_exit_2_with_written_false() {
    assert_eq!(oh(&[&"bogus"]), (2, failed("unsupported_command")));
    assert_eq!(oh(&[]), (2, failed("unsupported_command")));
    assert_eq!(oh(&[&"inspect", &"a", &"b"]), (2, failed("unsupported_command")));
    let d = common::tempdir();
    assert_eq!(oh(&[&"inspect", &d.path().join("missing.hwpx")]), (2, failed("input_io_error")));
    fs::write(d.path().join("x.hwpx"), b"not a zip").unwrap();
    assert_eq!(oh(&[&"validate", &d.path().join("x.hwpx")]), (2, failed("invalid_zip")));
}

#[test]
fn error_precedence_matches_legacy_per_command() {
    // invalid spec JSON + missing source: image/link family opens the source
    // before parsing the spec; the other edit commands parse first.
    let d = common::tempdir();
    let bad = d.path().join("bad.json");
    fs::write(&bad, b"{bad").unwrap();
    let src = d.path().join("nosrc.hwpx");
    let out = d.path().join("o.hwpx");
    for (cmd, code) in [("image-insert", "input_io_error"), ("image-replace", "input_io_error"), ("image", "input_io_error"),
        ("link", "input_io_error"), ("format", "invalid_format_spec"), ("paragraph", "invalid_paragraph_spec"),
        ("table", "invalid_table_spec"), ("style", "invalid_style_spec"), ("border", "invalid_border_spec")] {
        assert_eq!(oh(&[&cmd, &src, &"--spec", &bad, &"--output", &out]), (2, failed(code)), "{cmd}");
    }
    assert_eq!(oh(&[&"apply", &src, &"--plan", &bad, &"--output", &out]), (2, failed("invalid_plan")));
    assert_eq!(oh(&[&"create", &"--spec", &bad, &"--output", &out]), (2, failed("invalid_create_spec")));
    assert_eq!(names(d.path()), vec!["bad.json"]);
}

#[test]
fn oversize_contract_is_rejected() {
    let d = common::tempdir();
    let big = d.path().join("big.json");
    fs::write(&big, vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(oh(&[&"create", &"--spec", &big, &"--output", &d.path().join("o.hwpx")]), (2, failed("input_too_large")));
}

#[test]
fn create_then_inspect_round_trip() {
    let d = common::tempdir();
    let spec = d.path().join("create.json");
    fs::write(&spec, CREATE).unwrap();
    let dst = d.path().join("결과 문서.hwpx");
    let (rc, r) = oh(&[&"create", &"--spec", &spec, &"--output", &dst]);
    assert_eq!(rc, 0, "{r}");
    assert_eq!(r["native_validated"], false);
    assert_eq!(r["path"], json!(dst));
    assert_eq!(fs::read(&dst).unwrap(), SYNTH, "deterministic, identical to legacy create");
    let (rc, info) = oh(&[&"inspect", &dst]);
    assert_eq!(rc, 0);
    assert_eq!(info["tables"].as_array().unwrap().len(), 1);
    assert_eq!(info["cells"].as_array().unwrap().len(), 4);
    let (rc, v) = oh(&[&"validate", &dst]);
    assert_eq!((rc, &v["valid_supported_structure"]), (0, &json!(true)));
    // no parent directory → output_io_error, nothing written
    assert_eq!(oh(&[&"create", &"--spec", &spec, &"--output", &d.path().join("nodir").join("o.hwpx")]), (2, failed("output_io_error")));
}

#[test]
fn apply_noop_copy_and_no_clobber_of_source_or_existing_output() {
    let d = common::tempdir();
    let src = d.path().join("input.hwpx");
    fs::write(&src, SYNTH).unwrap();
    let rev = oh(&[&"inspect", &src]).1["revision"].as_str().unwrap().to_string();
    let plan = d.path().join("plan.json");
    fs::write(&plan, json!({"revision": rev, "operations": [{"op": "replace_text", "target": "Contents/section0.xml#t2", "expected": "항목", "text": "항목"}]}).to_string()).unwrap();
    let dst = d.path().join("noop.hwpx");
    let (rc, r) = oh(&[&"apply", &src, &"--plan", &plan, &"--output", &dst]);
    assert_eq!(rc, 0, "{r}");
    assert_eq!(r["changed_parts"], json!([]));
    assert_eq!(fs::read(&dst).unwrap(), SYNTH);
    for target in [&src, &dst] {
        assert_eq!(oh(&[&"apply", &src, &"--plan", &plan, &"--output", target]), (2, failed("target_exists")));
        assert_eq!(fs::read(target).unwrap(), SYNTH);
    }
    #[cfg(unix)]
    {
        let link = d.path().join("symlink.hwpx");
        std::os::unix::fs::symlink(&src, &link).unwrap();
        assert_eq!(oh(&[&"apply", &src, &"--plan", &plan, &"--output", &link]), (2, failed("target_exists")));
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(names(d.path()), vec!["input.hwpx", "noop.hwpx", "plan.json", "symlink.hwpx"]);
    }
    #[cfg(not(unix))]
    assert_eq!(names(d.path()), vec!["input.hwpx", "noop.hwpx", "plan.json"]);
}

#[test]
fn apply_real_edit_and_invalid_batches_never_write() {
    let d = common::tempdir();
    let src = d.path().join("input.hwpx");
    fs::write(&src, SYNTH).unwrap();
    let base: Value = serde_json::from_str(APPLY).unwrap();
    let plan = d.path().join("plan.json");
    for (code, op, field, value) in [("stale_target", 1, "expected", json!("wrong")), ("unknown_target", 1, "target", json!("missing")),
        ("invalid_operation", 1, "op", json!("delete")), ("invalid_xml_character", 1, "text", json!("\u{1}"))] {
        let mut p = base.clone();
        p["operations"][op][field] = value;
        fs::write(&plan, p.to_string()).unwrap();
        assert_eq!(oh(&[&"apply", &src, &"--plan", &plan, &"--output", &d.path().join("bad.hwpx")]), (2, failed(code)), "{code}");
    }
    let mut p = base.clone();
    p["revision"] = json!("0".repeat(64));
    fs::write(&plan, p.to_string()).unwrap();
    assert_eq!(oh(&[&"apply", &src, &"--plan", &plan, &"--output", &d.path().join("bad.hwpx")]), (2, failed("stale_document")));
    let mut p = base.clone();
    p["operations"][0]["extra"] = json!(1);
    fs::write(&plan, p.to_string()).unwrap();
    assert_eq!(oh(&[&"apply", &src, &"--plan", &plan, &"--output", &d.path().join("bad.hwpx")]), (2, failed("invalid_plan")));
    assert_eq!(names(d.path()), vec!["input.hwpx", "plan.json"], "no output, no stage files");
    assert_eq!(fs::read(&src).unwrap(), SYNTH);

    fs::write(&plan, APPLY).unwrap();
    let dst = d.path().join("edited.hwpx");
    let (rc, r) = oh(&[&"apply", &src, &"--plan", &plan, &"--output", &dst]);
    assert_eq!(rc, 0, "{r}");
    assert_eq!(r["changed_parts"], json!(["Contents/section0.xml"]));
    assert_eq!(r["sha256"], "8743f497982692cd2d7424bc76ed6f29101af04f2c135dbee144c999b5ee7d92");
    let texts: Vec<String> = oh(&[&"inspect", &dst]).1["text_nodes"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap().to_string()).collect();
    assert!(texts.contains(&"고친 셀 <확인> & 검토".to_string()));
    let (rc, diff) = oh(&[&"diff", &src, &dst]);
    assert_eq!((rc, &diff["content_parts_changed"]), (0, &json!(1)));
}

#[test]
fn hostile_zip_inputs_are_rejected_without_output() {
    // duplicate central entries / traversal names / not-a-zip
    let d = common::tempdir();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("invalid_zip", b"PK\x03\x04garbage".to_vec()),
        ("unsafe_member_path", zip_with(&[("../evil.xml", b"<a/>")])),
        ("unsafe_member_path", zip_with(&[("/abs.xml", b"<a/>")])),
    ];
    for (i, (code, bytes)) in cases.into_iter().enumerate() {
        let p = d.path().join(format!("h{i}.hwpx"));
        fs::write(&p, bytes).unwrap();
        assert_eq!(oh(&[&"inspect", &p]), (2, failed(code)), "case {i}");
    }
}

/// Minimal stored ZIP writer (std only) for hostile-input tests.
fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data { c ^= b as u32; for _ in 0..8 { c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 }; } }
        !c
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in entries {
        let off = out.len() as u32;
        let (n, crc, len) = (name.as_bytes(), crc32(data), data.len() as u32);
        out.extend_from_slice(b"PK\x03\x04"); out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        out.extend_from_slice(&crc.to_le_bytes()); out.extend_from_slice(&len.to_le_bytes()); out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(n.len() as u16).to_le_bytes()); out.extend_from_slice(&[0, 0]); out.extend_from_slice(n); out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02"); central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        central.extend_from_slice(&crc.to_le_bytes()); central.extend_from_slice(&len.to_le_bytes()); central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&(n.len() as u16).to_le_bytes()); central.extend_from_slice(&[0; 12]); central.extend_from_slice(&off.to_le_bytes()); central.extend_from_slice(n);
    }
    let (cd_off, cd_len) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06"); out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes()); out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes()); out.extend_from_slice(&cd_off.to_le_bytes()); out.extend_from_slice(&[0, 0]);
    out
}
