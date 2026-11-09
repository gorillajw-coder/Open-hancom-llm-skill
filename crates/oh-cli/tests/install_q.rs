//! `oh install` / `uninstall` / `doctor` through the real binary, always against a temporary
//! HOME (never the real client settings). Q06 (config missing/corrupt/other servers),
//! Q01 (install folder not writable), Q07 (installed copy finds its own library).
//! Expected values are typed by hand.
mod common;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Env {
    home: PathBuf,
}
impl Env {
    fn new(d: &Path) -> Env {
        let home = d.join("home 사용자");
        for s in ["tmp", "AppData/Roaming", "AppData/Local"] {
            fs::create_dir_all(home.join(s)).unwrap();
        }
        Env { home }
    }
    fn cmd(&self, exe: &Path) -> Command {
        let mut c = Command::new(exe);
        c.env_clear()
            .env("HOME", &self.home).env("USERPROFILE", &self.home)
            .env("APPDATA", self.home.join("AppData/Roaming")).env("LOCALAPPDATA", self.home.join("AppData/Local"))
            .env("TEMP", self.home.join("tmp")).env("TMP", self.home.join("tmp")).env("TMPDIR", self.home.join("tmp"))
            // a real Claude Desktop on the test machine must not trip the running-client guard
            .env("OH_CLIENT_PROCESS_NAME", "oh-test-no-such-client.exe");
        if cfg!(windows) {
            if let Some(s) = std::env::var_os("SystemRoot") { c.env("SystemRoot", s); }
        }
        c
    }
    fn oh(&self, args: &[&str]) -> (i32, Value) {
        self.oh_with(Path::new(env!("CARGO_BIN_EXE_oh")), args)
    }
    fn oh_with(&self, exe: &Path, args: &[&str]) -> (i32, Value) {
        let out = self.cmd(exe).args(args).output().unwrap();
        assert!(out.stderr.is_empty(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        (out.status.code().unwrap(), serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("stdout not JSON ({e}): {:?}", String::from_utf8_lossy(&out.stdout))))
    }
    fn root(&self) -> PathBuf {
        self.home.join("OH 설치")
    }
    fn wd(&self) -> PathBuf {
        self.home.join("OH 작업")
    }
    /// install with explicit root/workdir/library (the test binary has no library next to it)
    fn install(&self, client: &str, extra: &[&str]) -> (i32, Value) {
        let (root, wd, lib) = (self.root(), self.wd(), repo().join("library"));
        let mut a: Vec<&str> = vec!["install", "--client", client, "--root", root.to_str().unwrap(), "--workdir", wd.to_str().unwrap(), "--library", lib.to_str().unwrap()];
        a.extend_from_slice(extra);
        self.oh(&a)
    }
    fn claude_desktop_config(&self) -> PathBuf {
        if cfg!(windows) {
            self.home.join("AppData/Roaming/Claude/claude_desktop_config.json")
        } else if cfg!(target_os = "macos") {
            self.home.join("Library/Application Support/Claude/claude_desktop_config.json")
        } else {
            self.home.join(".config/Claude/claude_desktop_config.json")
        }
    }
}
fn bin_name() -> &'static str {
    if cfg!(windows) { "oh.exe" } else { "oh" }
}
fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<_> = fs::read_dir(d).map(|r| r.map(|e| e.unwrap().file_name().into_string().unwrap()).collect()).unwrap_or_default();
    v.sort();
    v
}
fn json_file(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}
fn backups(p: &Path) -> Vec<PathBuf> {
    let prefix = format!("{}.oh-backup-", p.file_name().unwrap().to_string_lossy());
    let mut v: Vec<_> = fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().path()).filter(|b| b.file_name().unwrap().to_string_lossy().starts_with(&prefix)).collect();
    v.sort();
    v
}
fn err_code(r: &(i32, Value)) -> &str {
    assert_eq!(r.0, 2, "{:?}", r.1);
    assert_eq!(r.1["written"], false);
    r.1["error"].as_str().unwrap()
}

#[test]
fn q06_no_config_creates_one_and_installed_copy_serves_mcp() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = e.claude_desktop_config();
    assert!(!cfg.exists());
    let r = e.install("claude-desktop", &[]);
    assert_eq!(r.0, 0, "{:?}", r.1);
    assert_eq!(r.1["backup"], Value::Null);
    assert_eq!(r.1["entry_replaced"], false);
    assert_eq!(r.1["verified"], true); // read back after writing
    assert_eq!(r.1["warnings"], json!([]));
    let root = e.root();
    let want_args = json!(["mcp", "--library", root.join("library"), "--workdir", e.wd()]);
    assert_eq!(json_file(&cfg), json!({"mcpServers": {"open-hancom": {"command": root.join(bin_name()), "args": want_args}}}));
    // installed files
    let mut got = names(&root);
    got.sort();
    assert_eq!(got, {
        let mut v = vec!["INSTALL.md", "PROMPT.md", "library", bin_name()];
        v.sort();
        v.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    });
    assert_eq!(fs::read_to_string(root.join("PROMPT.md")).unwrap(), fs::read_to_string(repo().join("PROMPT.md")).unwrap());
    assert_eq!(fs::read(root.join("library/manifest.toml")).unwrap(), fs::read(repo().join("library/manifest.toml")).unwrap());
    assert_eq!(names(&e.wd()), ["inbox", "outbox", "work"]);

    // Q07: the installed copy, started exactly as the client config says, loads its library and answers
    let entry = &json_file(&cfg)["mcpServers"]["open-hancom"];
    let mut child = e.cmd(Path::new(entry["command"].as_str().unwrap())).args(entry["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "oh_guide", "arguments": {}}})).unwrap();
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let resp: Value = serde_json::from_str(&line).unwrap();
    let body: Value = serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["library_loaded"], true);
    assert!(body["cards"].as_array().unwrap().iter().any(|c| c["id"] == "client-report-letter"));
    drop(stdin);
    assert!(child.wait().unwrap().success());

    // the installed copy also passes its own doctor (default library = next to the exe)
    let dr = e.oh_with(&root.join(bin_name()), &["doctor", "--workdir", e.wd().to_str().unwrap()]);
    assert_eq!(dr.0, 0, "{}", dr.1);
    assert_eq!(dr.1["ok"], true);
}

#[test]
fn q06_existing_config_is_backed_up_and_other_servers_survive() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = e.claude_desktop_config();
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    let original = r#"{
  "theme": "dark",
  "mcpServers": {
    "filesystem": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\a"], "env": {"K": "v"}},
    "other": {"command": "x"}
  },
  "numbers": [1, 2.5, 1e3]
}"#;
    fs::write(&cfg, original).unwrap();
    let r = e.install("claude-desktop", &[]);
    assert_eq!(r.0, 0, "{:?}", r.1);
    let b = backups(&cfg);
    assert_eq!(b.len(), 1);
    assert_eq!(fs::read_to_string(&b[0]).unwrap(), original, "backup is byte-identical to the original");
    assert_eq!(PathBuf::from(r.1["backup"].as_str().unwrap()), b[0]);
    let now = json_file(&cfg);
    let before: Value = serde_json::from_str(original).unwrap();
    assert_eq!(now["theme"], "dark");
    assert_eq!(now["numbers"], before["numbers"]);
    assert_eq!(now["mcpServers"]["filesystem"], before["mcpServers"]["filesystem"]);
    assert_eq!(now["mcpServers"]["other"], before["mcpServers"]["other"]);
    assert_eq!(now["mcpServers"].as_object().unwrap().len(), 3);
    assert_eq!(now["mcpServers"]["open-hancom"]["args"][0], "mcp");
    // re-install: entry replaced in place, nothing else lost, one more backup (of the first result)
    let first_result = fs::read_to_string(&cfg).unwrap();
    let r2 = e.install("claude-desktop", &[]);
    assert_eq!(r2.0, 0);
    assert_eq!(r2.1["entry_replaced"], true);
    assert_eq!(json_file(&cfg), now);
    let b2 = backups(&cfg);
    assert_eq!(b2.len(), 2);
    assert!(b2.iter().any(|p| fs::read_to_string(p).unwrap() == first_result));
}

#[test]
fn q06_corrupt_or_unexpected_json_is_never_overwritten() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("truncated", br#"{"mcpServers": {"a": {"command": "x"}"#.to_vec()),
        ("garbage", b"not json at all".to_vec()),
        ("array root", b"[1,2,3]".to_vec()),
        ("scalar root", b"42".to_vec()),
        ("mcpServers array", br#"{"mcpServers": []}"#.to_vec()),
        ("mcpServers string", br#"{"mcpServers": "x"}"#.to_vec()),
        ("invalid utf8", vec![b'{', 0xff, 0xfe, b'}']),
        ("trailing garbage", br#"{} extra"#.to_vec()),
    ];
    for (what, bytes) in cases {
        let d = common::tempdir();
        let e = Env::new(d.path());
        let cfg = e.claude_desktop_config();
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        fs::write(&cfg, &bytes).unwrap();
        let r = e.install("claude-desktop", &[]);
        assert_eq!(err_code(&r), "config_corrupt", "{what}: {:?}", r.1);
        assert!(r.1["detail"].as_str().unwrap().contains("건드리지 않았다"), "{what}");
        assert_eq!(fs::read(&cfg).unwrap(), bytes, "{what}: file changed");
        assert!(backups(&cfg).is_empty(), "{what}: backup made");
        assert!(!e.root().exists() && !e.wd().exists(), "{what}: partial install");
        // uninstall refuses too
        let u = e.oh(&["uninstall", "--client", "claude-desktop", "--root", e.root().to_str().unwrap()]);
        assert_eq!(err_code(&u), "config_corrupt", "{what}");
        assert_eq!(fs::read(&cfg).unwrap(), bytes, "{what}: uninstall changed file");
    }
}

#[test]
fn q06_empty_file_and_bom_are_accepted() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = e.claude_desktop_config();
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, "").unwrap();
    assert_eq!(e.install("claude-desktop", &[]).0, 0);
    assert_eq!(backups(&cfg).len(), 1);
    assert!(json_file(&cfg)["mcpServers"]["open-hancom"].is_object());
    let d2 = common::tempdir();
    let e2 = Env::new(d2.path());
    let cfg2 = e2.claude_desktop_config();
    fs::create_dir_all(cfg2.parent().unwrap()).unwrap();
    fs::write(&cfg2, b"\xef\xbb\xbf{\"mcpServers\":{\"a\":{\"command\":\"x\"}}}").unwrap();
    assert_eq!(e2.install("claude-desktop", &[]).0, 0);
    let now = json_file(&cfg2);
    assert_eq!(now["mcpServers"]["a"], json!({"command": "x"}));
}

#[test]
fn claude_code_config_keeps_unrelated_state_and_marks_stdio() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = e.home.join(".claude.json");
    fs::write(&cfg, r#"{"numStartups": 12, "projects": {"/x": {"allowedTools": []}}, "mcpServers": {"keep": {"type": "http", "url": "https://example.invalid"}}}"#).unwrap();
    let r = e.install("claude-code", &[]);
    assert_eq!(r.0, 0, "{:?}", r.1);
    let now = json_file(&cfg);
    assert_eq!(now["numStartups"], 12);
    assert_eq!(now["projects"], json!({"/x": {"allowedTools": []}}));
    assert_eq!(now["mcpServers"]["keep"], json!({"type": "http", "url": "https://example.invalid"}));
    assert_eq!(now["mcpServers"]["open-hancom"]["type"], "stdio");
    assert_eq!(PathBuf::from(r.1["config"].as_str().unwrap()), cfg);
    // CLAUDE_CONFIG_DIR is honoured
    let alt = d.path().join("alt-claude");
    fs::create_dir_all(&alt).unwrap();
    let out = e.cmd(Path::new(env!("CARGO_BIN_EXE_oh"))).env("CLAUDE_CONFIG_DIR", &alt)
        .args(["install", "--client", "claude-code", "--root", e.root().to_str().unwrap(), "--workdir", e.wd().to_str().unwrap(), "--library", repo().join("library").to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(json_file(&alt.join(".claude.json"))["mcpServers"]["open-hancom"].is_object());
}

#[test]
fn codex_toml_is_merged_without_losing_comments_or_servers() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = e.home.join(".codex/config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    let original = "# my codex settings\nmodel = \"gpt-5\"\n\n[mcp_servers.context7]\ncommand = \"npx\"\nargs = [\"-y\", \"@upstash/context7-mcp\"] # keep me\n\n[profiles.fast]\nmodel = \"x\"\n";
    fs::write(&cfg, original).unwrap();
    let r = e.install("codex", &[]);
    assert_eq!(r.0, 0, "{:?}", r.1);
    let now = fs::read_to_string(&cfg).unwrap();
    for keep in ["# my codex settings", "model = \"gpt-5\"", "[mcp_servers.context7]", "args = [\"-y\", \"@upstash/context7-mcp\"] # keep me", "[profiles.fast]"] {
        assert!(now.contains(keep), "lost {keep:?}:\n{now}");
    }
    assert!(now.contains("[mcp_servers.open-hancom]"), "{now}");
    assert!(now.contains("command = "), "{now}");
    let parsed: toml_check::Doc = toml_check::parse(&now);
    assert_eq!(parsed.args, ["mcp", "--library", e.root().join("library").to_str().unwrap(), "--workdir", e.wd().to_str().unwrap()]);
    assert_eq!(parsed.command, e.root().join(bin_name()).to_str().unwrap());
    let b = backups(&cfg);
    assert_eq!(b.len(), 1);
    assert_eq!(fs::read_to_string(&b[0]).unwrap(), original);
    // idempotent
    let r2 = e.install("codex", &[]);
    assert_eq!(r2.1["entry_replaced"], true);
    assert_eq!(fs::read_to_string(&cfg).unwrap().matches("[mcp_servers.open-hancom]").count(), 1);
    // new file when none exists
    let d3 = common::tempdir();
    let e3 = Env::new(d3.path());
    assert_eq!(e3.install("codex", &[]).0, 0);
    assert!(fs::read_to_string(e3.home.join(".codex/config.toml")).unwrap().contains("[mcp_servers.open-hancom]"));
}

/// Tiny TOML reader for the two keys we assert on (the test must not use the installer's own writer).
mod toml_check {
    pub struct Doc {
        pub command: String,
        pub args: Vec<String>,
    }
    fn unq(s: &str) -> String {
        let s = s.trim();
        let inner = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).or_else(|| s.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))).unwrap_or(s);
        inner.replace("\\\\", "\u{0}").replace("\\\"", "\"").replace('\u{0}', "\\")
    }
    pub fn parse(t: &str) -> Doc {
        let start = t.find("[mcp_servers.open-hancom]").expect("table");
        let body = &t[start + "[mcp_servers.open-hancom]".len()..];
        let body = body.split("\n[").next().unwrap();
        let mut command = String::new();
        let mut args = Vec::new();
        for l in body.lines() {
            if let Some(v) = l.trim().strip_prefix("command") { command = unq(v.trim_start().trim_start_matches('=')); }
            if let Some(v) = l.trim().strip_prefix("args") {
                let v = v.trim_start().trim_start_matches('=').trim().trim_start_matches('[').trim_end_matches(']');
                let mut cur = String::new();
                let (mut q, mut esc) = (None::<char>, false);
                for c in v.chars() {
                    match q {
                        None => if c == '"' || c == '\'' { q = Some(c); },
                        Some(open) => {
                            if esc { cur.push(c); esc = false; }
                            else if c == '\\' && open == '"' { esc = true; }
                            else if c == open { args.push(std::mem::take(&mut cur)); q = None; }
                            else { cur.push(c); }
                        }
                    }
                }
            }
        }
        Doc { command, args }
    }
}

#[test]
fn codex_corrupt_toml_is_never_overwritten() {
    for (what, text) in [("unterminated table", "[mcp_servers\ncommand = 1\n"), ("bad value", "model = \n"), ("mcp_servers scalar", "mcp_servers = 5\n"), ("dup key", "a = 1\na = 2\n")] {
        let d = common::tempdir();
        let e = Env::new(d.path());
        let cfg = e.home.join(".codex/config.toml");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        fs::write(&cfg, text).unwrap();
        let r = e.install("codex", &[]);
        assert_eq!(err_code(&r), "config_corrupt", "{what}: {:?}", r.1);
        assert_eq!(fs::read_to_string(&cfg).unwrap(), text, "{what}");
        assert!(backups(&cfg).is_empty());
        assert!(!e.root().exists(), "{what}");
    }
}

#[test]
fn chatgpt_desktop_needs_an_explicit_config_and_says_so() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let r = e.install("chatgpt-desktop", &[]);
    assert_eq!(err_code(&r), "config_path_required");
    assert!(r.1["detail"].as_str().unwrap().contains("수동"));
    assert!(!e.root().exists() && !e.wd().exists(), "nothing installed without a config path");
    let cfg = d.path().join("chatgpt-mcp.json");
    let r = e.install("chatgpt-desktop", &["--config", cfg.to_str().unwrap()]);
    assert_eq!(r.0, 0, "{:?}", r.1);
    assert!(r.1["format_note"].as_str().unwrap().contains("미확인"));
    assert!(json_file(&cfg)["mcpServers"]["open-hancom"].is_object());
    // a .toml path is written as TOML (docs: the app shares Codex's config.toml)
    let toml = d.path().join("shared.toml");
    fs::write(&toml, "# keep\n[mcp_servers.a]\ncommand = \"x\"\n").unwrap();
    assert_eq!(e.install("chatgpt-desktop", &["--config", toml.to_str().unwrap()]).0, 0);
    let t = fs::read_to_string(&toml).unwrap();
    assert!(t.contains("# keep") && t.contains("[mcp_servers.a]") && t.contains("[mcp_servers.open-hancom]"), "{t}");
}

#[test]
fn explicit_config_overrides_the_default_location() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = d.path().join("elsewhere/한글 설정.json");
    let r = e.install("claude-desktop", &["--config", cfg.to_str().unwrap()]);
    assert_eq!(r.0, 0, "{:?}", r.1);
    assert!(cfg.exists());
    assert!(!e.claude_desktop_config().exists());
}

#[test]
fn uninstall_is_symmetric_and_keeps_user_files() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let cfg = e.claude_desktop_config();
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
    assert_eq!(e.install("claude-desktop", &[]).0, 0);
    fs::write(e.wd().join("outbox/내 결과물.hwpx"), b"keep").unwrap();
    let u = e.oh(&["uninstall", "--client", "claude-desktop", "--root", e.root().to_str().unwrap()]);
    assert_eq!(u.0, 0, "{:?}", u.1);
    assert_eq!(u.1["entry_removed"], true);
    assert_eq!(u.1["verified"], true);
    assert_eq!(json_file(&cfg), json!({"mcpServers": {"other": {"command": "x"}}}));
    assert_eq!(backups(&cfg).len(), 2); // install backup + uninstall backup
    assert!(e.root().join("library").exists(), "files stay without --remove-files");
    // second uninstall: nothing to do, no new backup
    let u2 = e.oh(&["uninstall", "--client", "claude-desktop", "--root", e.root().to_str().unwrap()]);
    assert_eq!((u2.0, &u2.1["entry_removed"]), (0, &json!(false)));
    assert_eq!(backups(&cfg).len(), 2);
    // with --remove-files the install folder goes, the work folder never does
    let u3 = e.oh(&["uninstall", "--client", "claude-desktop", "--root", e.root().to_str().unwrap(), "--remove-files"]);
    assert_eq!(u3.0, 0, "{:?}", u3.1);
    assert!(!e.root().exists(), "{:?}", names(&e.root()));
    assert_eq!(fs::read(e.wd().join("outbox/내 결과물.hwpx")).unwrap(), b"keep");
    assert_eq!(u3.1["workdir_kept"], true);
    // uninstall with no config file at all
    let d2 = common::tempdir();
    let e2 = Env::new(d2.path());
    let u = e2.oh(&["uninstall", "--client", "codex"]);
    assert_eq!((u.0, &u.1["entry_removed"]), (0, &json!(false)));
    // codex uninstall keeps other tables and comments
    let d3 = common::tempdir();
    let e3 = Env::new(d3.path());
    let c3 = e3.home.join(".codex/config.toml");
    fs::create_dir_all(c3.parent().unwrap()).unwrap();
    fs::write(&c3, "# c\n[mcp_servers.a]\ncommand = \"x\"\n").unwrap();
    assert_eq!(e3.install("codex", &[]).0, 0);
    assert_eq!(e3.oh(&["uninstall", "--client", "codex"]).0, 0);
    let t = fs::read_to_string(&c3).unwrap();
    assert!(t.contains("# c") && t.contains("[mcp_servers.a]") && !t.contains("open-hancom"), "{t}");
}

// ---- Q01: folders that cannot be written ----
struct Undo(Option<Box<dyn FnOnce()>>);
impl Drop for Undo {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() { f(); }
    }
}
#[cfg(unix)]
fn deny_writes(dir: &Path) -> Undo {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o500)).unwrap();
    let d = dir.to_path_buf();
    Undo(Some(Box::new(move || { let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o700)); })))
}
#[cfg(windows)]
fn deny_writes(dir: &Path) -> Undo {
    let st = Command::new("icacls").arg(dir).args(["/deny", "*S-1-1-0:(OI)(CI)(WD,AD)"]).stdout(Stdio::null()).status().unwrap();
    assert!(st.success(), "icacls deny failed");
    let d = dir.to_path_buf();
    Undo(Some(Box::new(move || { let _ = Command::new("icacls").arg(&d).args(["/remove:d", "*S-1-1-0"]).stdout(Stdio::null()).status(); })))
}

#[test]
fn q01_unwritable_install_folder_leaves_no_partial_install_and_config_untouched() {
    for variant in ["root exists read-only", "root parent read-only", "root blocked by a file"] {
        let d = common::tempdir();
        let e = Env::new(d.path());
        let cfg = e.claude_desktop_config();
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        let original = br#"{"mcpServers":{"a":{"command":"x"}}}"#;
        fs::write(&cfg, original).unwrap();
        let (root, _undo) = match variant {
            "root exists read-only" => {
                let r = d.path().join("ro-root");
                fs::create_dir(&r).unwrap();
                (r.clone(), deny_writes(&r))
            }
            "root parent read-only" => {
                let p = d.path().join("ro-parent");
                fs::create_dir(&p).unwrap();
                (p.join("root"), deny_writes(&p))
            }
            _ => {
                let f = d.path().join("a-file");
                fs::write(&f, b"x").unwrap();
                (f.join("root"), Undo(None))
            }
        };
        let (wd, lib) = (e.wd(), repo().join("library"));
        let r = e.oh(&["install", "--client", "claude-desktop", "--root", root.to_str().unwrap(), "--workdir", wd.to_str().unwrap(), "--library", lib.to_str().unwrap()]);
        assert_eq!(err_code(&r), "install_dir_unwritable", "{variant}: {:?}", r.1);
        assert_eq!(fs::read(&cfg).unwrap(), original, "{variant}: config changed");
        assert!(backups(&cfg).is_empty(), "{variant}");
        if variant != "root blocked by a file" {
            assert_eq!(names(&root), Vec::<String>::new(), "{variant}: files or stage left in the install folder");
        }
    }
}

#[test]
fn q01_unwritable_work_folder_is_a_coded_error_and_nothing_is_installed() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let blocker = d.path().join("blocker");
    fs::write(&blocker, b"x").unwrap();
    let wd = blocker.join("work");
    let (root, lib) = (e.root(), repo().join("library"));
    let r = e.oh(&["install", "--client", "claude-desktop", "--root", root.to_str().unwrap(), "--workdir", wd.to_str().unwrap(), "--library", lib.to_str().unwrap()]);
    assert_eq!(err_code(&r), "workdir_unwritable", "{:?}", r.1);
    assert!(!e.claude_desktop_config().exists());
    assert_eq!(names(&root), Vec::<String>::new());
}

#[test]
fn install_refuses_a_library_that_fails_its_hash_check() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let lib = d.path().join("lib-copy");
    fn copy(a: &Path, b: &Path) {
        fs::create_dir_all(b).unwrap();
        for x in fs::read_dir(a).unwrap() {
            let x = x.unwrap();
            if x.file_type().unwrap().is_dir() { copy(&x.path(), &b.join(x.file_name())); } else { fs::copy(x.path(), b.join(x.file_name())).unwrap(); }
        }
    }
    copy(&repo().join("library"), &lib);
    let mut t = fs::read(lib.join("defaults.toml")).unwrap();
    t.push(b'\n');
    fs::write(lib.join("defaults.toml"), t).unwrap();
    let (root, wd) = (e.root(), e.wd());
    let r = e.oh(&["install", "--client", "claude-desktop", "--root", root.to_str().unwrap(), "--workdir", wd.to_str().unwrap(), "--library", lib.to_str().unwrap()]);
    assert_eq!(err_code(&r), "library_invalid");
    assert!(r.1["detail"].as_str().unwrap().contains("manifest_hash_mismatch"));
    assert!(!root.exists() && !e.claude_desktop_config().exists());
    // doctor reports the same defect and exits non-zero
    let dr = e.oh(&["doctor", "--library", lib.to_str().unwrap(), "--workdir", wd.to_str().unwrap()]);
    assert_eq!(dr.0, 2);
    assert_eq!(dr.1["ok"], false);
    let lib_check = dr.1["checks"].as_array().unwrap().iter().find(|c| c["check"] == "library").unwrap();
    assert_eq!(lib_check["ok"], false);
    assert!(lib_check["error"].as_str().unwrap().contains("manifest_hash_mismatch"));
}

#[test]
fn doctor_reports_each_check_and_cleans_its_temp_files() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let (lib, wd) = (repo().join("library"), e.wd());
    let r = e.oh(&["doctor", "--library", lib.to_str().unwrap(), "--workdir", wd.to_str().unwrap()]);
    assert_eq!(r.0, 0, "{}", r.1);
    let checks = r.1["checks"].as_array().unwrap();
    let ids: Vec<&str> = checks.iter().map(|c| c["check"].as_str().unwrap()).collect();
    assert_eq!(ids, ["version", "library", "library_layers", "workdir", "synthetic_document", "client_registration"]);
    assert_eq!(checks[0]["detail"]["oh"], env!("CARGO_PKG_VERSION"));
    let library = &checks[1]["detail"];
    assert_eq!(library["anonymized"], true);
    assert!(library["files_hash_verified"].as_u64().unwrap() >= 6);
    assert!(library["cards"].as_array().unwrap().iter().any(|c| c == "client-report-letter"));
    assert_eq!(checks[2]["detail"]["layers"][0]["name"], "base");
    assert_eq!(checks[4]["detail"]["text_nodes"], 6);
    assert_eq!(names(&wd.join("outbox")), Vec::<String>::new(), "probe file removed");
    assert_eq!(names(&e.home.join("tmp")), Vec::<String>::new(), "temporary outbox removed");
    // an unwritable work folder fails the workdir check and the exit code
    let blocker = d.path().join("blocker");
    fs::write(&blocker, b"x").unwrap();
    let bad = e.oh(&["doctor", "--library", lib.to_str().unwrap(), "--workdir", blocker.join("w").to_str().unwrap()]);
    assert_eq!(bad.0, 2);
    let wc = bad.1["checks"].as_array().unwrap().iter().find(|c| c["check"] == "workdir").unwrap();
    assert_eq!(wc["ok"], false);
    assert!(wc["error"].as_str().unwrap().contains("폴더"));
}

#[test]
fn admin_commands_reject_bad_arguments() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    for args in [
        vec!["install"],
        vec!["install", "--client", "bing"],
        vec!["install", "--client"],
        vec!["install", "--client", "codex", "--client", "codex"],
        vec!["install", "--client", "codex", "--bogus", "1"],
        vec!["install", "--client", "codex", "stray"],
        vec!["install", "--client", "codex", "--remove-files"],
        vec!["uninstall", "--client", "codex", "--workdir", "x"],
        vec!["doctor", "--client", "codex"],
        vec!["mcp", "--library"],
        vec!["mcp", "extra"],
    ] {
        let r = e.oh(&args);
        assert_eq!(err_code(&r), "invalid_arguments", "{args:?}: {:?}", r.1);
    }
    assert_eq!(names(&e.home), ["AppData", "tmp"], "no side effects");
}

/// Windows: a running client app rewrites its config on exit, so install/uninstall refuse while
/// it runs (`client_running`); `--force-running` proceeds with a warning. A copy of ping.exe
/// under a unique image name stands in for Claude.exe.
#[cfg(windows)]
#[test]
fn claude_desktop_running_is_refused_unless_forced() {
    let d = common::tempdir();
    let e = Env::new(d.path());
    let fake = d.path().join("oh-fake-claude-app.exe");
    fs::copy(Path::new(&std::env::var("SystemRoot").unwrap()).join("System32/PING.EXE"), &fake).unwrap();
    let mut app = Command::new(&fake).args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let run = |args: &[&str]| {
        let out = e.cmd(Path::new(env!("CARGO_BIN_EXE_oh"))).env("OH_CLIENT_PROCESS_NAME", "oh-fake-claude-app.exe").args(args).output().unwrap();
        (out.status.code().unwrap(), serde_json::from_slice::<Value>(&out.stdout).unwrap())
    };
    let (root, wd, lib) = (e.root(), e.wd(), repo().join("library"));
    let base = ["install", "--client", "claude-desktop", "--root", root.to_str().unwrap(), "--workdir", wd.to_str().unwrap(), "--library", lib.to_str().unwrap()];
    let r = run(&base);
    assert_eq!(r.0, 2, "{}", r.1);
    assert_eq!(r.1["error"], "client_running");
    assert!(r.1["detail"].as_str().unwrap().contains("트레이까지 완전히 종료한 뒤 다시 실행"));
    assert!(!e.claude_desktop_config().exists() && !root.exists(), "nothing written");
    let mut forced = base.to_vec();
    forced.push("--force-running");
    let r = run(&forced);
    assert_eq!(r.0, 0, "{}", r.1);
    assert_eq!(r.1["verified"], true);
    assert!(r.1["warnings"][0].as_str().unwrap().starts_with("client_running"));
    // doctor warns while it runs
    let dr = run(&["doctor", "--library", root.join("library").to_str().unwrap(), "--workdir", wd.to_str().unwrap()]);
    let reg = dr.1["checks"].as_array().unwrap().iter().find(|c| c["check"] == "client_registration").unwrap();
    assert_eq!(reg["detail"]["claude-desktop"]["running"], true, "{reg}");
    assert!(reg["detail"]["claude-desktop"]["warning"].as_str().unwrap().contains("덮어써질 수 있다"));
    let u = run(&["uninstall", "--client", "claude-desktop", "--root", root.to_str().unwrap()]);
    assert_eq!(u.1["error"], "client_running");
    let u = run(&["uninstall", "--client", "claude-desktop", "--root", root.to_str().unwrap(), "--force-running"]);
    assert_eq!((u.0, &u.1["verified"]), (0, &json!(true)), "{}", u.1);
    let _ = app.kill();
    let _ = app.wait();
    // not running any more: no refusal, no warning
    let r = run(&base);
    assert_eq!((r.0, &r.1["warnings"]), (0, &json!([])), "{}", r.1);
}
