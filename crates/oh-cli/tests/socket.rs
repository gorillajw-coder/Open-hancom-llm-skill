//! Library socket (D12): personal > connected packs > base, hash checks, conflicts, zip packs,
//! disconnect, and paths with spaces and Hangul. Real `oh library` / `oh mcp` processes.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
mod common;

fn repo_lib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}
fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
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
/// Write manifest.toml listing every file under `dir` (except itself).
fn manifest(dir: &Path, version: &str, anonymized: bool) {
    fn walk(root: &Path, d: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.retain(|f| f != "manifest.toml");
    files.sort();
    let mut s = format!("version = \"{version}\"\nanonymized = {anonymized}\n");
    for f in files {
        let kind = match f.split('/').next().unwrap() {
            "cards" => "card",
            "templates" => "template",
            "frames" => "frame",
            "examples" => "example",
            _ if f == "defaults.toml" => "defaults",
            _ => "regulation",
        };
        s.push_str(&format!("\n[[file]]\npath = \"{f}\"\nsha256 = \"{}\"\nkind = \"{kind}\"\n", sha(&std::fs::read(dir.join(&f)).unwrap())));
    }
    std::fs::write(dir.join("manifest.toml"), s).unwrap();
}
/// A pack (or personal library) with the letter card re-titled and, optionally, a pack-only card
/// that extends the base frame `official-letter`.
fn make_layer(dir: &Path, letter_title: &str, extra_card: Option<&str>, anonymized: bool) {
    std::fs::create_dir_all(dir.join("cards")).unwrap();
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::copy(repo_lib().join("templates/official-letter.hwpx"), dir.join("templates/official-letter.hwpx")).unwrap();
    let base = std::fs::read_to_string(repo_lib().join("cards/client-report-letter.toml")).unwrap();
    std::fs::write(dir.join("cards/client-report-letter.toml"), base.replace("title = \"발주기관 앞 보고·요청 공문\"", &format!("title = \"{letter_title}\""))).unwrap();
    if let Some(id) = extra_card {
        let c = base
            .replace("id = \"client-report-letter\"", &format!("id = \"{id}\""))
            .replace("title = \"발주기관 앞 보고·요청 공문\"", "title = \"팀 회신 공문\"")
            .replace("triggers = [", "triggers = [\"팀 회신 공문\", ");
        std::fs::write(dir.join(format!("cards/{id}.toml")), c).unwrap();
    }
    manifest(dir, "1.0.0", anonymized);
}
fn install_root(d: &Path) -> PathBuf {
    let root = d.join("설치 폴더");
    copy_dir(&repo_lib(), &root.join("library"));
    root
}
fn oh(args: &[&str], root: &Path) -> (Value, bool) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_oh"));
    c.args(args).arg("--root").arg(root);
    let o = c.output().unwrap();
    let out = String::from_utf8(o.stdout).unwrap();
    (serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}")), o.status.success())
}
fn add(root: &Path, pack: &Path, name: Option<&str>) -> (Value, bool) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_oh"));
    c.args(["library", "add", "--path"]).arg(pack).arg("--root").arg(root);
    if let Some(n) = name {
        c.args(["--name", n]);
    }
    let o = c.output().unwrap();
    let out = String::from_utf8(o.stdout).unwrap();
    (serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}")), o.status.success())
}
fn layer<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["library"]["layers"].as_array().unwrap().iter().find(|l| l["name"] == name).unwrap_or_else(|| panic!("no layer {name}: {v}"))
}
fn mcp(root: &Path, wd: &Path, calls: &[(&str, Value)]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oh"))
        .args(["mcp", "--library"]).arg(root.join("library")).arg("--workdir").arg(wd)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();
    let mut w = child.stdin.take().unwrap();
    let mut r = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}})).unwrap();
    r.read_line(&mut line).unwrap();
    let mut out = Vec::new();
    for (i, (name, args)) in calls.iter().enumerate() {
        writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        out.push(serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap());
    }
    drop(w);
    let _ = child.wait();
    out
}

#[test]
fn layers_priority_conflicts_and_disconnect() {
    let d = common::tempdir();
    let root = install_root(d.path());
    // the base alone: same as before (single --library compatibility)
    let (v, ok) = oh(&["library", "list"], &root);
    assert!(ok, "{v}");
    assert_eq!(v["library"]["layers"].as_array().unwrap().len(), 1);
    assert_eq!(v["library"]["conflicts"], json!([]));

    let pack = d.path().join("팀 라이브러리 팩");
    make_layer(&pack, "팀 공문", Some("team-reply-letter"), true);
    let (v, ok) = add(&root, &pack, Some("팀-공용"));
    assert!(ok, "{v}");
    assert_eq!(v["connected"], "팀-공용");
    assert_eq!(layer(&v, "팀-공용")["status"], "loaded");
    let mut cards: Vec<&str> = layer(&v, "팀-공용")["cards"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    cards.sort();
    assert_eq!(cards, ["client-report-letter", "team-reply-letter"]); // the pack card extends the base frame
    assert_eq!(v["library"]["conflicts"], json!([{"kind": "card", "id": "client-report-letter", "used": "팀-공용", "shadowed": ["base"]}]));
    let s = std::fs::read_to_string(root.join("sockets.toml")).unwrap();
    assert!(s.contains("name = \"팀-공용\"") && s.contains("manifest_sha256 = "), "{s}");

    // personal library (not anonymized: allowed for personal only) beats the pack
    let personal = root.join("personal-library");
    make_layer(&personal, "내 공문", None, false);
    let (v, _) = oh(&["library", "list"], &root);
    assert_eq!(layer(&v, "personal")["status"], "loaded");
    assert_eq!(v["library"]["conflicts"], json!([{"kind": "card", "id": "client-report-letter", "used": "personal", "shadowed": ["팀-공용", "base"]}]));
    assert_eq!(v["library"]["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect::<Vec<_>>(), ["personal", "팀-공용", "base"]);

    // MCP shows the source layer and makes a real document from the pack card
    let wd = d.path().join("작업 폴더");
    let u = |v: &str| json!({"value": v, "source": "사용자"});
    let l = |v: &str| json!({"value": v, "source": "llm"});
    let plan = json!({"documents": [{"card": "team-reply-letter", "slots": {
        "doc_date": u("2026-03-02"), "doc_no": u("개좋소-2026-040"), "recipient": l("김밥천국공사 사장"),
        "doc_title": l("자료 요청에 대한 회신"), "body": l("관련: 김밥천국공사-2026-011\n요청하신 자료를 붙임과 같이 제출합니다."), "attachments": l("월간 진도 보고서 1부.")}}]});
    let out = mcp(&root, &wd, &[
        ("oh_guide", json!({})),
        ("oh_library_search", json!({"query": "공문"})),
        ("oh_card_get", json!({"card": "client-report-letter"})),
    ]);
    let g = &out[0];
    assert!(g["cards"].as_array().unwrap().iter().any(|c| c["id"] == "team-reply-letter" && c["layer"] == "팀-공용"));
    assert!(g["cards"].as_array().unwrap().iter().any(|c| c["id"] == "client-report-letter" && c["layer"] == "personal" && c["title"] == "내 공문"));
    assert_eq!(g["library_layers"]["conflicts"][0]["used"], "personal");
    assert!(out[1]["hits"].as_array().unwrap().iter().any(|h| h["id"] == "team-reply-letter" && h["layer"] == "팀-공용"), "{}", out[1]);
    assert_eq!(out[1]["conflicts"][0]["shadowed"], json!(["팀-공용", "base"]));
    assert_eq!(out[2]["layer"], "personal");
    // draft + finalize in one server process (cases live in memory)
    let mut child = Command::new(env!("CARGO_BIN_EXE_oh"))
        .args(["mcp", "--library"]).arg(root.join("library")).arg("--workdir").arg(&wd)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();
    let mut w = child.stdin.take().unwrap();
    let mut r = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id: u64, name: &str, args: Value| -> Value {
        writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let c = call(1, "oh_case_open", json!({"summary": "팀 회신 공문"}));
    let cid = c["case_id"].as_str().unwrap();
    let dr = call(2, "oh_draft", json!({"case_id": cid, "plan": plan}));
    let did = dr["draft_id"].as_str().unwrap_or_else(|| panic!("{dr}")).to_string();
    let fin = call(3, "oh_finalize", json!({"draft_id": did, "output_name": "팀 회신 공문.hwpx"}));
    let path = fin["path"].as_str().unwrap_or_else(|| panic!("{fin}")).to_string();
    let texts: Vec<String> = oh_document::Package::from_bytes(std::fs::read(&path).unwrap()).unwrap().inspection.text_nodes.iter().map(|n| n.text.clone()).collect();
    assert!(texts.iter().any(|t| t == "붙임  월간 진도 보고서 1부.  끝."), "{texts:?}");
    drop(w);
    let _ = child.wait();

    // disconnect: the pack card disappears, the letter falls back to personal, then base
    let (v, ok) = oh(&["library", "remove", "--name", "팀-공용"], &root);
    assert!(ok, "{v}");
    assert_eq!(v["files_deleted"], false);
    assert!(pack.join("manifest.toml").exists());
    assert_eq!(v["library"]["conflicts"], json!([{"kind": "card", "id": "client-report-letter", "used": "personal", "shadowed": ["base"]}]));
    std::fs::remove_dir_all(&personal).unwrap();
    let (v, _) = oh(&["library", "list"], &root);
    assert_eq!(v["library"]["layers"].as_array().unwrap().len(), 1);
    let (v, ok) = oh(&["library", "remove", "--name", "팀-공용"], &root);
    assert!(!ok && v["error"] == "pack_not_connected");
}

#[test]
fn hash_checks_and_rejections() {
    let d = common::tempdir();
    let root = install_root(d.path());
    let pack = d.path().join("pack a");
    make_layer(&pack, "팀 공문", Some("team-reply-letter"), true);
    // not anonymized: a shared pack is refused
    let raw = d.path().join("raw pack");
    make_layer(&raw, "x", None, false);
    let (v, ok) = add(&root, &raw, Some("raw"));
    assert!(!ok && v["error"] == "library_not_anonymized", "{v}");
    // a file that does not match the manifest: refused at connection
    let bad = d.path().join("bad pack");
    make_layer(&bad, "x", None, true);
    std::fs::write(bad.join("cards/client-report-letter.toml"), "id = \"tampered\"").unwrap();
    let (v, ok) = add(&root, &bad, Some("bad"));
    assert!(!ok && v["error"] == "manifest_hash_mismatch", "{v}");
    assert!(!root.join("sockets.toml").exists(), "nothing recorded on refusal");
    // a card that does not bind (template hash) makes the pack fail as a whole
    let unbound = d.path().join("unbound");
    make_layer(&unbound, "x", None, true);
    let c = std::fs::read_to_string(unbound.join("cards/client-report-letter.toml")).unwrap();
    let h = c.lines().find(|l| l.starts_with("template_sha256")).unwrap().to_string();
    std::fs::write(unbound.join("cards/client-report-letter.toml"), c.replace(&h, &format!("template_sha256 = \"{}\"", "0".repeat(64)))).unwrap();
    manifest(&unbound, "1.0.0", true);
    let (v, ok) = add(&root, &unbound, Some("unbound"));
    assert!(!ok && v["error"] == "pack_rejected", "{v}");
    // bad names
    let (v, ok) = add(&root, &pack, Some("base"));
    assert!(!ok && v["error"] == "invalid_arguments", "{v}");

    let (_, ok) = add(&root, &pack, Some("a"));
    assert!(ok);
    let (v, ok) = add(&root, &pack, Some("a2"));
    assert!(!ok && v["error"] == "pack_already_connected", "{v}");
    // tampered after connection: skipped and reported; base cards still work
    let t = pack.join("templates/official-letter.hwpx");
    let mut b = std::fs::read(&t).unwrap();
    let n = b.len();
    b[n / 2] ^= 1;
    std::fs::write(&t, &b).unwrap();
    let (v, _) = oh(&["library", "list"], &root);
    assert!(layer(&v, "a")["status"].as_str().unwrap().starts_with("manifest_hash_mismatch"), "{v}");
    assert!(layer(&v, "base")["cards"].as_array().unwrap().iter().any(|c| c == "client-report-letter"));
    // re-signed by its maker (new manifest): the pin refuses it until connected again
    b[n / 2] ^= 1;
    std::fs::write(&t, &b).unwrap();
    std::fs::create_dir_all(pack.join("regulations")).unwrap();
    std::fs::write(pack.join("regulations/team-note.md"), "팀 메모").unwrap();
    manifest(&pack, "1.0.1", true);
    let (v, _) = oh(&["library", "list"], &root);
    assert!(layer(&v, "a")["status"].as_str().unwrap().starts_with("pack_changed"), "{v}");
    let (v, ok) = add(&root, &pack, Some("a"));
    assert!(ok && v["replaced"] == true && v["version"] == "1.0.1", "{v}");
    assert_eq!(layer(&v, "a")["status"], "loaded");
    // doctor reports the layers
    let o = Command::new(env!("CARGO_BIN_EXE_oh")).args(["doctor", "--library"]).arg(root.join("library")).arg("--workdir").arg(d.path().join("wd")).output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let c = v["checks"].as_array().unwrap().iter().find(|c| c["check"] == "library_layers").unwrap();
    assert_eq!(c["ok"], true, "{c}");
    assert_eq!(c["detail"]["layers"].as_array().unwrap().len(), 2);
}

#[test]
fn zip_pack_with_top_folder() {
    let d = common::tempdir();
    let root = install_root(d.path());
    let src = d.path().join("src");
    make_layer(&src, "팀 공문", Some("team-reply-letter"), true);
    let zp = d.path().join("공용 팩 v1.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        fn put(z: &mut zip::ZipWriter<std::fs::File>, o: zip::write::SimpleFileOptions, root: &Path, d: &Path) {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    put(z, o, root, &p);
                } else {
                    z.start_file(format!("team-pack/{}", p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")), o).unwrap();
                    z.write_all(&std::fs::read(&p).unwrap()).unwrap();
                }
            }
        }
        put(&mut z, o, &src, &src);
        z.finish().unwrap();
    }
    let (v, ok) = add(&root, &zp, None);
    assert!(ok, "{v}");
    assert_eq!(v["connected"], "공용-팩-v1", "{v}"); // default name: file stem, spaces to '-'
    assert_eq!(layer(&v, "공용-팩-v1")["status"], "loaded");
    let (v, _) = oh(&["library", "list"], &root);
    assert!(layer(&v, "공용-팩-v1")["cards"].as_array().unwrap().iter().any(|c| c == "team-reply-letter"));
    // a zip without a manifest is refused
    let empty = d.path().join("empty.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&empty).unwrap());
        z.start_file("a/b/readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
    }
    let (v, ok) = add(&root, &empty, Some("empty"));
    assert!(!ok && v["error"] == "invalid_pack", "{v}");
}

#[test]
fn new_pack_may_not_knock_out_a_loaded_layer_and_zip_ignores_unlisted_members() {
    let d = common::tempdir();
    let root = install_root(d.path());
    // personal card omits the frame slot `via`
    let personal = root.join("personal-library");
    make_layer(&personal, "내 공문", None, false);
    let c = std::fs::read_to_string(personal.join("cards/client-report-letter.toml")).unwrap();
    std::fs::write(personal.join("cards/client-report-letter.toml"), c.replace("extends = \"official-letter\"", "extends = \"official-letter\"\nomit = [\"via\"]")).unwrap();
    manifest(&personal, "1.0.0", false);
    let (v, _) = oh(&["library", "list"], &root);
    assert_eq!(layer(&v, "personal")["status"], "loaded", "{v}");
    // a pack that replaces the frame without `via` would break the personal card
    let pack = d.path().join("novia");
    std::fs::create_dir_all(pack.join("frames")).unwrap();
    let f = std::fs::read_to_string(repo_lib().join("frames/official-letter.toml")).unwrap();
    let start = f.find("[[slot]]\nid = \"via\"").unwrap();
    let end = start + f[start..].find("\n[[slot]]\nid = \"cc\"").unwrap() + 1;
    let f = f[..start].to_string() + &f[end..];
    let cs = f.find("[[cell]]\nid = \"via\"").unwrap();
    let ce = cs + f[cs..].find("\n[[cell]]\nid = \"cc\"").unwrap() + 1;
    std::fs::write(pack.join("frames/official-letter.toml"), f[..cs].to_string() + &f[ce..]).unwrap();
    manifest(&pack, "1.0.0", true);
    let (v, ok) = add(&root, &pack, Some("novia"));
    assert!(!ok && v["error"] == "pack_rejected" && v["detail"].as_str().unwrap().contains("personal"), "{v}");
    assert!(!root.join("sockets.toml").exists());

    // zip: a big member outside the manifest is never inflated (reviewer #1)
    let src = d.path().join("src");
    make_layer(&src, "팀 공문", Some("team-reply-letter"), true);
    let zp = d.path().join("junk.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for rel in ["manifest.toml", "cards/client-report-letter.toml", "cards/team-reply-letter.toml", "templates/official-letter.hwpx"] {
            z.start_file(rel, o).unwrap();
            z.write_all(&std::fs::read(src.join(rel)).unwrap()).unwrap();
        }
        z.start_file("junk/zeros.bin", o.large_file(true)).unwrap();
        let zeros = vec![0u8; 1 << 20];
        for _ in 0..40 {
            z.write_all(&zeros).unwrap(); // 40 MB, over the 16 MB per-file cap
        }
        z.finish().unwrap();
    }
    std::fs::remove_dir_all(&personal).unwrap();
    let (v, ok) = add(&root, &zp, Some("junk"));
    assert!(ok, "{v}");
    assert_eq!(layer(&v, "junk")["status"], "loaded");
}
