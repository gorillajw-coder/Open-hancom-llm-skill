//! Byte-API tests for oh-document. Fixtures are embedded with include_bytes!,
//! so these tests perform no filesystem I/O at run time. Golden JSON files were
//! produced by the legacy P0 `open-hancom-core` binary (see fixtures/README.md).
use oh_document::Package;
use serde_json::Value;

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic.hwpx");

fn golden(raw: &str) -> Value {
    serde_json::from_str(raw).expect("golden json")
}

// ---- slice 1: read-only ----

#[test]
fn inspect_from_bytes_matches_legacy_cli() {
    let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    let got = serde_json::to_value(&p.inspection).unwrap();
    assert_eq!(got, golden(include_str!("fixtures/synthetic.inspect.json")));
}

#[test]
fn validate_from_bytes_matches_legacy_cli() {
    let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    assert_eq!(p.validate_supported(), golden(include_str!("fixtures/synthetic.validate.json")));
}

#[test]
fn diff_from_bytes_matches_legacy_cli() {
    let a = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    let b = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    assert_eq!(a.diff(&b), golden(include_str!("fixtures/synthetic.diff-self.json")));
}

#[test]
fn schema_matches_legacy_cli() {
    assert_eq!(oh_document::discovery(), golden(include_str!("fixtures/schema.json")));
}

#[test]
fn from_bytes_rejects_non_zip_and_oversize_without_io() {
    assert_eq!(Package::from_bytes(b"not a zip".to_vec()).err(), Some("invalid_zip"));
    let huge = vec![0u8; oh_document::MAX_INPUT as usize + 1];
    assert_eq!(Package::from_bytes(huge).err(), Some("input_too_large"));
}

#[test]
fn revision_is_sha256_of_input_bytes() {
    use sha2::{Digest, Sha256};
    let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    assert_eq!(p.inspection.revision, format!("{:x}", Sha256::digest(FIXTURE)));
}

// ---- slice 2: prepare apply (no I/O) ----

fn plan(raw: &str) -> oh_document::Plan {
    oh_document::parse_contract(raw.as_bytes(), "invalid_plan").unwrap()
}
const APPLY: &str = include_str!("fixtures/apply.json");
const APPLY_SHA256: &str = "8743f497982692cd2d7424bc76ed6f29101af04f2c135dbee144c999b5ee7d92";

fn sha(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

#[test]
fn apply_bytes_matches_legacy_output_and_receipt() {
    let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    let out = p.apply_bytes(plan(APPLY)).unwrap();
    assert_eq!(sha(&out.bytes), APPLY_SHA256);
    assert_eq!(out.receipt, golden(include_str!("fixtures/apply.receipt.json")));
    assert!(out.receipt.get("path").is_none(), "core receipt must not carry a path");
    // candidate reopens and carries the new text
    let again = Package::from_bytes(out.bytes).unwrap();
    let texts: Vec<_> = again.inspection.text_nodes.iter().map(|n| n.text.as_str()).collect();
    assert!(texts.contains(&"고친 셀 <확인> & 검토") && texts.contains(&"빈 칸 채움"));
}

#[test]
fn apply_noop_returns_input_bytes_unchanged() {
    let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    let rev = &p.inspection.revision;
    let noop = format!(r#"{{"revision":"{rev}","operations":[{{"op":"replace_text","target":"Contents/section0.xml#t2","expected":"항목","text":"항목"}}]}}"#);
    let out = p.apply_bytes(plan(&noop)).unwrap();
    assert_eq!(out.bytes, FIXTURE);
    assert_eq!(out.receipt["changed_parts"], serde_json::json!([]));
}

#[test]
fn apply_rejections_are_atomic_and_keep_legacy_codes() {
    let base: Value = serde_json::from_str(APPLY).unwrap();
    let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("stale_document", Box::new(|p| p["revision"] = "0".repeat(64).into())),
        ("stale_target", Box::new(|p| p["operations"][1]["expected"] = "wrong".into())),
        ("unknown_target", Box::new(|p| p["operations"][1]["target"] = "missing".into())),
        ("invalid_operation", Box::new(|p| p["operations"][1]["op"] = "delete".into())),
        ("duplicate_target", Box::new(|p| { let first = p["operations"][0].clone(); p["operations"][1] = first; })),
        ("invalid_operations", Box::new(|p| p["operations"] = serde_json::json!([]))),
        ("invalid_xml_character", Box::new(|p| p["operations"][1]["text"] = "\u{1}".into())),
    ];
    for (code, mutate) in cases {
        let mut v = base.clone();
        mutate(&mut v);
        let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
        // the first operation is valid: a later failure must reject the whole batch
        assert_eq!(p.apply_bytes(plan(&v.to_string())).err(), Some(code), "{code}");
    }
}

#[test]
fn apply_preserves_unchanged_members_raw_bytes_and_metadata() {
    use std::io::{Cursor, Read};
    let p = Package::from_bytes(FIXTURE.to_vec()).unwrap();
    let out = p.apply_bytes(plan(APPLY)).unwrap();
    let mut a = zip::ZipArchive::new(Cursor::new(FIXTURE)).unwrap();
    let mut b = zip::ZipArchive::new(Cursor::new(&out.bytes)).unwrap();
    assert_eq!(a.len(), b.len());
    for i in 0..a.len() {
        let (name, raw_a, meta_a) = { let mut f = a.by_index_raw(i).unwrap(); let mut r = vec![]; f.read_to_end(&mut r).unwrap(); (f.name().to_string(), r, (f.compression(), f.crc32(), f.last_modified())) };
        let mut f = b.by_index_raw(i).unwrap();
        assert_eq!(f.name(), name, "member order");
        if name == "Contents/section0.xml" { continue; }
        let mut raw_b = vec![]; f.read_to_end(&mut raw_b).unwrap();
        assert_eq!(raw_b, raw_a, "raw bytes of {name}");
        assert_eq!((f.compression(), f.crc32(), f.last_modified()), meta_a, "metadata of {name}");
    }
}

// ---- slice 4: create_bytes ----

#[test]
fn create_bytes_matches_legacy_output_and_receipt() {
    let spec = oh_document::parse_contract(include_bytes!("fixtures/create.json"), "invalid_create_spec").unwrap();
    let out = oh_document::create_bytes(spec).unwrap();
    assert_eq!(out.bytes, FIXTURE, "deterministic and identical to legacy create");
    assert_eq!(out.receipt, golden(include_str!("fixtures/create.receipt.json")));
    assert!(out.receipt.get("path").is_none());
}

#[test]
fn create_bytes_rejects_invalid_geometry() {
    let mut v: Value = serde_json::from_str(include_str!("fixtures/create.json")).unwrap();
    v["page"]["left"] = 40000.into();
    v["page"]["right"] = 40000.into();
    let spec = oh_document::parse_contract(v.to_string().as_bytes(), "invalid_create_spec").unwrap();
    assert_eq!(oh_document::create_bytes(spec).err(), Some("invalid_page_geometry"));
}

// ---- slice 5: remaining edit commands (goldens from legacy binary, see scripts/gen-bytes-fixtures.py) ----

const MEDIA: &[u8] = include_bytes!("fixtures/media.hwpx");

macro_rules! cmd_case {
    ($name:literal, $src:expr, $method:ident, $code:literal) => {{
        let manifest: Value = golden(include_str!("fixtures/cmd/manifest.json"));
        let p = Package::from_bytes($src.to_vec()).unwrap();
        let spec = oh_document::parse_contract(include_bytes!(concat!("fixtures/cmd/", $name, ".json")), $code).unwrap();
        let out = p.$method(spec).unwrap_or_else(|e| panic!("{}: {e}", $name));
        assert_eq!(sha(&out.bytes), manifest[$name]["sha256"].as_str().unwrap(), "{} bytes", $name);
        assert_eq!(out.receipt, golden(include_str!(concat!("fixtures/cmd/", $name, ".receipt.json"))), "{} receipt", $name);
        assert!(out.receipt.get("path").is_none());
        Package::from_bytes(out.bytes).expect("candidate reopens");
        // stale revision must be rejected before any work
        let mut v: Value = serde_json::from_str(include_str!(concat!("fixtures/cmd/", $name, ".json"))).unwrap();
        v["revision"] = "0".repeat(64).into();
        let stale = oh_document::parse_contract(v.to_string().as_bytes(), $code).unwrap();
        assert_eq!(p.$method(stale).err(), Some("stale_document"), "{} stale", $name);
    }};
}

#[test] fn format_bytes_matches_legacy() { cmd_case!("format", FIXTURE, format_bytes, "invalid_format_spec") }
#[test] fn paragraph_bytes_matches_legacy() { cmd_case!("paragraph", FIXTURE, paragraph_bytes, "invalid_paragraph_spec") }
#[test] fn table_bytes_matches_legacy() { cmd_case!("table", FIXTURE, table_bytes, "invalid_table_spec") }
#[test] fn border_bytes_matches_legacy() { cmd_case!("border", FIXTURE, border_bytes, "invalid_border_spec") }
#[test] fn style_bytes_matches_legacy() { cmd_case!("style", FIXTURE, style_bytes, "invalid_style_spec") }
#[test] fn link_bytes_matches_legacy() { cmd_case!("link", MEDIA, link_bytes, "invalid_link_spec") }
#[test] fn image_bytes_matches_legacy() { cmd_case!("image", MEDIA, image_bytes, "invalid_image_spec") }
#[test] fn image_replace_bytes_matches_legacy() { cmd_case!("image-replace", MEDIA, image_replace_bytes, "invalid_image_replace_spec") }
#[test] fn image_insert_bytes_matches_legacy() { cmd_case!("image-insert", MEDIA, image_insert_bytes, "invalid_image_insert_spec") }
