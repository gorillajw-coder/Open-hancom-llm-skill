//! `oh install` / `oh uninstall`: copy binary + library into a per-user folder and merge an
//! `open-hancom` entry into a client's MCP config. No admin rights, no registry, no
//! network. Existing config is backed up first; a corrupt config is never overwritten.
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use toml_edit::{value, Array, DocumentMut, Item, Table};

pub const NAME: &str = "open-hancom";
pub type Fail = (&'static str, String);
fn fail<T>(code: &'static str, detail: impl Into<String>) -> Result<T, Fail> {
    Err((code, detail.into()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Client {
    ClaudeDesktop,
    ChatgptDesktop,
    Codex,
    ClaudeCode,
}
impl Client {
    pub fn parse(s: &str) -> Option<Client> {
        Some(match s {
            "claude-desktop" => Client::ClaudeDesktop,
            "chatgpt-desktop" => Client::ChatgptDesktop,
            "codex" => Client::Codex,
            "claude-code" => Client::ClaudeCode,
            _ => return None,
        })
    }
    pub const ALL: [(&'static str, Client); 4] = [
        ("claude-desktop", Client::ClaudeDesktop),
        ("chatgpt-desktop", Client::ChatgptDesktop),
        ("codex", Client::Codex),
        ("claude-code", Client::ClaudeCode),
    ];
    /// Codex uses TOML. ChatGPT desktop has no verified default; its docs say the app shares
    /// Codex's `config.toml`, so an explicit `--config *.toml` is written as TOML, anything
    /// else as `mcpServers` JSON.
    fn is_toml(self, path: &Path) -> bool {
        self == Client::Codex || (self == Client::ChatgptDesktop && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml")))
    }
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}
fn home() -> Result<PathBuf, Fail> {
    env_path(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map_or_else(|| fail("home_unknown", "홈 폴더를 알 수 없다. --root/--workdir/--config를 지정한다"), Ok)
}

pub fn default_root() -> Result<PathBuf, Fail> {
    if cfg!(windows) {
        if let Some(l) = env_path("LOCALAPPDATA") {
            return Ok(l.join("OpenHancom"));
        }
        return Ok(home()?.join("AppData").join("Local").join("OpenHancom"));
    }
    Ok(home()?.join(".local/share/open-hancom"))
}
pub fn default_workdir() -> Result<PathBuf, Fail> {
    if cfg!(windows) {
        let docs = oh_host::documents_dir().map_or_else(|| home().map(|h| h.join("Documents")), Ok)?;
        return Ok(docs.join("OpenHancom"));
    }
    Ok(home()?.join("OpenHancom"))
}

/// Default config location per client; `None` when no verified location exists.
pub fn default_config(c: Client) -> Result<Option<PathBuf>, Fail> {
    Ok(match c {
        Client::ClaudeDesktop => Some(if cfg!(windows) {
            env_path("APPDATA").map_or_else(|| home().map(|h| h.join("AppData").join("Roaming")), Ok)?.join("Claude").join("claude_desktop_config.json")
        } else if cfg!(target_os = "macos") {
            home()?.join("Library/Application Support/Claude/claude_desktop_config.json")
        } else {
            home()?.join(".config/Claude/claude_desktop_config.json")
        }),
        Client::ClaudeCode => Some(env_path("CLAUDE_CONFIG_DIR").map_or_else(|| home().map(|h| h.join(".claude.json")), |d| Ok(d.join(".claude.json")))?),
        Client::Codex => Some(env_path("CODEX_HOME").map_or_else(|| home().map(|h| h.join(".codex")), Ok)?.join("config.toml")),
        Client::ChatgptDesktop => None,
    })
}

/// Store/MSIX Claude Desktop: the packaged app's %APPDATA% is virtualized to
/// `%LOCALAPPDATA%\Packages\<PFN>\LocalCache\Roaming`. Reports (anthropics/claude-code#26073) say the app
/// reads that copy while the in-app "Edit Config" button opens the real %APPDATA% file, and that the
/// two are not synced; a user also reported the virtualized file not being picked up. Evidence is
/// community-only and mixed, so when a `Claude_*` package folder exists both files are managed
/// (virtualized first). `local`/`appdata` are the LOCALAPPDATA/APPDATA folders.
///
/// Newer MSIX builds (seen on 4.23118, 2026-11) keep user data outside the package in
/// `%LOCALAPPDATA%\Claude-Data\<pfn-lowercase>\` (README there says so; pref
/// `userDataOutsidePackageAnswer`) and read `claude_desktop_config.json` from that folder only.
/// When such a folder exists it is managed too and listed first, since it is the one the app reads.
pub fn claude_desktop_paths(local: &Path, appdata: &Path) -> Vec<PathBuf> {
    let real = appdata.join("Claude").join("claude_desktop_config.json");
    let mut data: Vec<PathBuf> = std::fs::read_dir(local.join("Claude-Data"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("claude_") && e.path().is_dir())
        .map(|e| e.path().join("claude_desktop_config.json"))
        .collect();
    data.sort();
    let mut pkgs: Vec<PathBuf> = std::fs::read_dir(local.join("Packages"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_") && e.path().is_dir())
        .map(|e| e.path())
        .collect();
    pkgs.sort();
    let mut out = data;
    out.extend(pkgs.into_iter().map(|d| d.join("LocalCache").join("Roaming").join("Claude").join("claude_desktop_config.json")));
    out.push(real);
    out
}

/// Default config locations per client (more than one only for an MSIX Claude Desktop).
pub fn default_configs(c: Client) -> Result<Vec<PathBuf>, Fail> {
    if c == Client::ClaudeDesktop && cfg!(windows) {
        let local = env_path("LOCALAPPDATA").map_or_else(|| home().map(|h| h.join("AppData").join("Local")), Ok)?;
        let appdata = env_path("APPDATA").map_or_else(|| home().map(|h| h.join("AppData").join("Roaming")), Ok)?;
        return Ok(claude_desktop_paths(&local, &appdata));
    }
    Ok(default_config(c)?.into_iter().collect())
}

const CONFIG_MAX: u64 = 64 * 1024 * 1024;

/// Existing config text (BOM stripped). `Ok(None)` if the file does not exist.
fn read_config(path: &Path) -> Result<Option<String>, Fail> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = oh_host::bounded_read(path, CONFIG_MAX).map_err(|c| (if c == "input_io_error" { "config_unreadable" } else { c }, path.display().to_string()))?;
    let s = String::from_utf8(raw).map_err(|_| ("config_corrupt", format!("{}: UTF-8이 아니다. 파일은 건드리지 않았다", path.display())))?;
    Ok(Some(s.strip_prefix('\u{feff}').unwrap_or(&s).to_string()))
}

fn corrupt<T>(path: &Path, why: &str) -> Result<T, Fail> {
    fail("config_corrupt", format!("{}: {why}. 파일은 건드리지 않았다. 고친 뒤 다시 실행한다", path.display()))
}

/// Entry for JSON clients.
fn json_entry(c: Client, exe: &Path, args: &[String]) -> Value {
    let mut e = json!({"command": exe.to_string_lossy(), "args": args});
    if c == Client::ClaudeCode {
        e["type"] = json!("stdio");
    }
    e
}

/// Insert or remove the entry in `text`; returns (new text, existed_before).
pub fn merge(c: Client, path: &Path, text: Option<&str>, entry: Option<(&Path, &[String])>) -> Result<(String, bool), Fail> {
    if c.is_toml(path) {
        let mut doc: DocumentMut = match text.filter(|t| !t.trim().is_empty()) {
            Some(t) => t.parse().map_err(|e| ("config_corrupt", format!("{}: TOML 오류 {e}. 파일은 건드리지 않았다", path.display().to_string())))?,
            None => DocumentMut::new(),
        };
        let root = doc.as_table_mut();
        if !root.contains_key("mcp_servers") {
            if entry.is_none() {
                return Ok((doc.to_string(), false));
            }
            let mut t = Table::new();
            t.set_implicit(true);
            root.insert("mcp_servers", Item::Table(t));
        }
        let Some(servers) = root.get_mut("mcp_servers").and_then(Item::as_table_like_mut) else { return corrupt(path, "mcp_servers가 테이블이 아니다") };
        let existed = servers.contains_key(NAME);
        match entry {
            Some((exe, args)) => {
                let mut t = Table::new();
                t.insert("command", value(exe.to_string_lossy().to_string()));
                let mut arr = Array::new();
                for a in args {
                    arr.push(a.as_str());
                }
                t.insert("args", value(arr));
                servers.insert(NAME, Item::Table(t));
            }
            None => {
                servers.remove(NAME);
            }
        }
        Ok((doc.to_string(), existed))
    } else {
        let mut root: Value = match text.filter(|t| !t.trim().is_empty()) {
            Some(t) => serde_json::from_str(t).map_err(|e| ("config_corrupt", format!("{}: JSON 오류 {e}. 파일은 건드리지 않았다", path.display())))?,
            None => Value::Object(Map::new()),
        };
        let Some(obj) = root.as_object_mut() else { return corrupt(path, "최상위가 JSON 객체가 아니다") };
        if !obj.contains_key("mcpServers") {
            if entry.is_none() {
                return Ok((serde_json::to_string_pretty(&root).unwrap() + "\n", false));
            }
            obj.insert("mcpServers".into(), Value::Object(Map::new()));
        }
        let Some(servers) = obj.get_mut("mcpServers").and_then(Value::as_object_mut) else { return corrupt(path, "mcpServers가 객체가 아니다") };
        let existed = servers.contains_key(NAME);
        match entry {
            Some((exe, args)) => {
                servers.insert(NAME.into(), json_entry(c, exe, args));
            }
            None => {
                servers.remove(NAME);
            }
        }
        Ok((serde_json::to_string_pretty(&root).unwrap() + "\n", existed))
    }
}

/// Back up `path` (if present) and replace it with `new_text` via a same-folder temp file.
fn write_config(path: &Path, new_text: &str) -> Result<Option<PathBuf>, Fail> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| ("config_write_failed", format!("{}: {e}", dir.display())))?;
    }
    let mut backup = None;
    if path.exists() {
        let stamp = oh_mcp::now_stamp();
        let base = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let mut n = 0;
        let b = loop {
            let name = if n == 0 { format!("{base}.oh-backup-{stamp}") } else { format!("{base}.oh-backup-{stamp}-{n}") };
            let b = path.with_file_name(name);
            if !b.exists() {
                break b;
            }
            n += 1;
        };
        std::fs::copy(path, &b).map_err(|e| ("config_write_failed", format!("백업 실패 {}: {e}", b.display())))?;
        backup = Some(b);
    }
    let tmp = path.with_file_name(format!(".oh-tmp-{}-{}", std::process::id(), oh_mcp::now_stamp()));
    let res = std::fs::write(&tmp, new_text).and_then(|_| {
        if let Ok(m) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, m.permissions());
        }
        std::fs::rename(&tmp, path)
    });
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return fail("config_write_failed", format!("{}: {e}", path.display()));
    }
    Ok(backup)
}

pub struct Opts {
    pub client: Client,
    pub config: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub workdir: Option<PathBuf>,
    pub library: Option<PathBuf>,
    pub remove_files: bool,
    /// Proceed although the client app is running (a warning is still reported).
    pub force_running: bool,
}

/// Process image of the client app whose running copy rewrites its config on exit
/// (Claude Desktop, MSIX included: `claude.exe`). Only Claude Desktop on Windows is checked.
/// `OH_CLIENT_PROCESS_NAME` replaces the image name and turns the path filter off (tests use a
/// name nothing else runs under).
fn client_process(c: Client) -> Option<(String, bool)> {
    if !(c == Client::ClaudeDesktop && cfg!(windows)) {
        return None;
    }
    Some(match std::env::var("OH_CLIENT_PROCESS_NAME").ok().filter(|v| !v.trim().is_empty()) {
        Some(n) => (n, false),
        None => ("claude.exe".into(), true),
    })
}
/// `tasklist /FO CSV /NH` output: is a process with this image name listed? Locale-independent
/// (the "no tasks" message is localized; only CSV rows whose first field matches count).
pub fn tasklist_has(output: &str, image: &str) -> bool {
    output.lines().any(|l| l.trim_start().strip_prefix('"').and_then(|r| r.split('"').next()).is_some_and(|n| n.eq_ignore_ascii_case(image)))
}
/// Claude Code's native binary is also `claude.exe` (`%USERPROFILE%\.local\bin`, or under
/// `.claude`); everything else, including an unreadable path, counts as the desktop app.
pub fn is_claude_code_path(p: &str) -> bool {
    let l = p.trim().to_ascii_lowercase().replace('/', "\\");
    !l.is_empty() && (l.ends_with("\\.local\\bin\\claude.exe") || l.contains("\\.claude\\"))
}
pub enum Running {
    No,
    Yes(String),
    /// The check itself failed; reported, never refused on.
    Unknown(String),
}
pub fn client_running(c: Client) -> Option<Running> {
    let (image, filter) = client_process(c)?;
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let run = |exe: PathBuf, args: &[&str]| {
        std::process::Command::new(exe).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success())
    };
    let Some(out) = run(sys.join("System32").join("tasklist.exe"), &["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"]) else {
        return Some(Running::Unknown("tasklist 실행 실패".into()));
    };
    if !tasklist_has(&String::from_utf8_lossy(&out.stdout), &image) {
        return Some(Running::No);
    }
    if !filter {
        return Some(Running::Yes(image));
    }
    // which claude.exe: the desktop app or Claude Code? (own-user processes report their path)
    let ps = sys.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    match run(ps, &["-NoProfile", "-NonInteractive", "-Command", "Get-Process -Name claude -ErrorAction SilentlyContinue | ForEach-Object { '<' + $_.Path + '>' }"]) {
        None => Some(Running::Yes(format!("{image} (경로 확인 실패, 앱으로 간주)"))),
        Some(o) => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let paths: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix('<').and_then(|r| r.strip_suffix('>'))).collect();
            if !paths.is_empty() && paths.iter().all(|p| is_claude_code_path(p)) {
                Some(Running::No) // only Claude Code sessions
            } else {
                Some(Running::Yes(image))
            }
        }
    }
}
const RUNNING_MSG: &str = "클라이언트 앱이 실행 중이다. 실행 중인 앱은 종료할 때 설정 파일을 다시 써서 등록이 사라질 수 있다. 트레이까지 완전히 종료한 뒤 다시 실행한다";
/// Refuse while the client runs unless forced; forced runs and failed checks carry a warning.
fn running_guard(c: Client, force: bool) -> Result<Vec<String>, Fail> {
    match client_running(c) {
        Some(Running::Yes(image)) if !force => fail("client_running", format!("{image}: {RUNNING_MSG} (강제로 진행하려면 --force-running, 그래도 덮어써질 수 있다)")),
        Some(Running::Yes(image)) => Ok(vec![format!("client_running: {image} 실행 중에 --force-running으로 진행했다. 앱을 트레이까지 종료한 뒤 oh doctor로 registered를 다시 확인한다")]),
        Some(Running::Unknown(why)) => Ok(vec![format!("client_running_unknown: 앱 실행 여부를 확인하지 못했다({why}). 앱이 켜져 있으면 등록이 덮어써질 수 있다")]),
        _ => Ok(vec![]),
    }
}
/// Read the config back after writing: is our entry there (install) / gone (uninstall)?
fn entry_present(c: Client, path: &Path) -> Option<bool> {
    match read_config(path) {
        Ok(Some(t)) => merge(c, path, Some(&t), None).ok().map(|(_, existed)| existed),
        Ok(None) => Some(false),
        Err(_) => None,
    }
}

fn resolve_configs(o: &Opts) -> Result<Vec<PathBuf>, Fail> {
    if let Some(p) = &o.config {
        return Ok(vec![p.clone()]);
    }
    let v = default_configs(o.client)?;
    match v.is_empty() {
        false => Ok(v),
        true => fail(
            "config_path_required",
            "ChatGPT 데스크톱의 설정 파일 위치는 확인된 근거가 없어 자동으로 정하지 않는다(문서상 앱의 설정 > MCP servers > Add server > STDIO로 등록하며 Codex의 config.toml과 설정을 공유한다고 한다). --config <경로>를 주거나 앱에서 수동으로 등록한다. 서버 명령: <설치 폴더>/oh, 인수: mcp --library <설치 폴더>/library --workdir <작업 폴더>. 설정 파일은 바꾸지 않았다",
        ),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}
fn bin_name() -> &'static str {
    if cfg!(windows) { "oh.exe" } else { "oh" }
}

pub fn install(o: &Opts) -> Result<Value, Fail> {
    let configs = resolve_configs(o)?;
    let warnings = running_guard(o.client, o.force_running)?;
    // 1. Validate everything that can fail before touching the disk.
    let texts: Vec<Option<String>> = configs.iter().map(|c| read_config(c)).collect::<Result<_, _>>()?;
    let root = match &o.root { Some(r) => r.clone(), None => default_root()? };
    let workdir = match &o.workdir { Some(w) => w.clone(), None => default_workdir()? };
    let exe = std::env::current_exe().map_err(|e| ("install_source_missing", e.to_string()))?;
    let lib_src = match &o.library {
        Some(l) => l.clone(),
        None => exe.parent().map(|p| p.join("library")).ok_or(("install_source_missing", "실행 파일 폴더 없음".to_string()))?,
    };
    let lib = oh_library::Library::load(&oh_library::DirSource::new(&lib_src)).map_err(|e| ("library_invalid", format!("{}: {e}", lib_src.display())))?;
    let exe_dst = root.join(bin_name());
    let lib_dst = root.join("library");
    let args: Vec<String> = vec!["mcp".into(), "--library".into(), lib_dst.to_string_lossy().into(), "--workdir".into(), workdir.to_string_lossy().into()];
    // Dry merge: a structurally bad config must fail now, before any copy.
    let mut merged = Vec::new();
    for (c, t) in configs.iter().zip(&texts) {
        merged.push(merge(o.client, c, t.as_deref(), Some((&exe_dst, &args)))?);
    }

    // 2. Folders (a failure here leaves no installed files).
    std::fs::create_dir_all(&root).map_err(|e| ("install_dir_unwritable", format!("{}: {e}", root.display())))?;
    for sub in ["inbox", "work", "outbox"] {
        let d = workdir.join(sub);
        std::fs::create_dir_all(&d).map_err(|e| ("workdir_unwritable", format!("{}: {e}", d.display())))?;
    }

    // 3. Stage next to the final location, then swap in.
    let stage = root.join(format!(".install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let stage_result = (|| -> std::io::Result<()> {
        std::fs::create_dir(&stage)?;
        std::fs::create_dir(stage.join("prev"))?;
        if !same_file(&exe, &exe_dst) {
            std::fs::copy(&exe, stage.join(bin_name()))?;
        }
        if !same_file(&lib_src, &lib_dst) {
            let put = |rel: &str, bytes: &[u8]| -> std::io::Result<()> {
                let p = stage.join("library").join(rel);
                if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
                std::fs::write(p, bytes)
            };
            let src = oh_library::DirSource::new(&lib_src);
            put("manifest.toml", &oh_library::Source::read(&src, "manifest.toml").map_err(|e| std::io::Error::other(e.to_string()))?)?;
            for f in &lib.manifest.files {
                put(&f.path, lib.file(&f.path).ok_or_else(|| std::io::Error::other("library file missing"))?)?;
            }
        }
        std::fs::write(stage.join("PROMPT.md"), oh_mcp::PROMPT)?;
        std::fs::write(stage.join("INSTALL.md"), oh_mcp::INSTALL_DOC)?;
        Ok(())
    })();
    if let Err(e) = stage_result {
        let _ = std::fs::remove_dir_all(&stage);
        return fail("install_dir_unwritable", format!("{}: {e}", root.display()));
    }
    let mut moved: Vec<String> = Vec::new();
    let mut names = vec!["library", "PROMPT.md", "INSTALL.md"];
    names.insert(0, bin_name());
    for n in names {
        let from = stage.join(n);
        if !from.exists() {
            continue; // skipped (same file)
        }
        let to = root.join(n);
        let step = (|| -> std::io::Result<()> {
            if to.exists() {
                std::fs::rename(&to, stage.join("prev").join(n))?;
                moved.push(n.to_string());
            }
            std::fs::rename(&from, &to)
        })();
        if let Err(e) = step {
            // Roll back what was moved aside, then clean up.
            for m in &moved {
                let _ = std::fs::remove_file(root.join(m)).or_else(|_| std::fs::remove_dir_all(root.join(m)));
                let _ = std::fs::rename(stage.join("prev").join(m), root.join(m));
            }
            let _ = std::fs::remove_dir_all(&stage);
            return fail("install_dir_unwritable", format!("{}: {e} (사용 중인 파일이면 클라이언트를 종료한다)", to.display()));
        }
    }
    let _ = std::fs::remove_dir_all(&stage);

    // 4. Register with the client last.
    let mut written = Vec::new();
    for (c, (new_text, existed)) in configs.iter().zip(&merged) {
        let backup = write_config(c, new_text)?;
        written.push(json!({"config": c, "backup": backup, "entry_replaced": existed, "verified": entry_present(o.client, c) == Some(true)}));
    }
    let verified = written.iter().all(|w| w["verified"] == true);
    Ok(json!({
        "client": client_name(o.client), "config": configs[0], "backup": written[0]["backup"], "entry_replaced": merged[0].1, "configs": written,
        "verified": verified, "warnings": warnings,
        "root": root, "binary": exe_dst, "library": lib_dst, "workdir": workdir,
        "library_version": lib.manifest.version,
        "format_note": if o.client == Client::ChatgptDesktop { Some("ChatGPT 데스크톱의 설정 파일 위치·형식은 미확인. 지정한 경로에 (.toml이면 TOML, 아니면 mcpServers JSON)으로 기록했다. 앱이 인식하는지 직접 확인한다") } else { None },
        "next": "클라이언트를 다시 시작하고 oh doctor로 점검한다",
    }))
}

pub fn client_name(c: Client) -> &'static str {
    Client::ALL.iter().find(|(_, x)| *x == c).map(|(n, _)| *n).unwrap()
}

pub fn uninstall(o: &Opts) -> Result<Value, Fail> {
    let configs = resolve_configs(o)?;
    let warnings = running_guard(o.client, o.force_running)?;
    let config = configs[0].clone();
    let mut removed = false;
    let mut backup = None;
    let mut plan = Vec::new();
    for c in &configs {
        if let Some(t) = read_config(c)? {
            let (new_text, existed) = merge(o.client, c, Some(&t), None)?;
            if existed {
                plan.push((c.clone(), new_text));
            }
        }
    }
    for (c, new_text) in &plan {
        let b = write_config(c, new_text)?;
        if !removed {
            backup = b;
        }
        removed = true;
    }
    let removed_from: Vec<&PathBuf> = plan.iter().map(|(c, _)| c).collect();
    let mut remaining: Vec<String> = Vec::new();
    let mut removed_files = Vec::new();
    if o.remove_files {
        let root = match &o.root { Some(r) => r.clone(), None => default_root()? };
        for n in [bin_name(), "library", "PROMPT.md", "INSTALL.md"] {
            let p = root.join(n);
            if p.symlink_metadata().is_err() {
                continue;
            }
            match std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir_all(&p)) {
                Ok(()) => removed_files.push(p.display().to_string()),
                Err(e) => remaining.push(format!("{}: {e}", p.display())),
            }
        }
        let _ = std::fs::remove_dir(&root); // only if empty
    }
    // read back: the entry must be gone from every location
    let verified = configs.iter().all(|c| entry_present(o.client, c) == Some(false));
    Ok(json!({"client": client_name(o.client), "config": config, "entry_removed": removed, "entry_removed_from": removed_from, "backup": backup,
              "files_removed": removed_files, "files_remaining": remaining, "workdir_kept": true, "verified": verified, "warnings": warnings}))
}

/// Read-only: does the client's default config already contain our entry? (One row per location.)
pub fn registered(c: Client) -> Value {
    let v = registered_rows(c);
    match (client_running(c), v) {
        (Some(Running::Yes(image)), Value::Object(mut m)) => {
            m.insert("running".into(), json!(true));
            m.insert("warning".into(), json!(format!("{image} 실행 중: 앱이 종료하면서 설정 파일을 다시 쓰면 등록이 덮어써질 수 있다. 트레이까지 종료한 뒤 oh doctor로 다시 확인한다")));
            Value::Object(m)
        }
        (Some(Running::No), Value::Object(mut m)) => {
            m.insert("running".into(), json!(false));
            Value::Object(m)
        }
        (Some(Running::Unknown(why)), Value::Object(mut m)) => {
            m.insert("running".into(), Value::Null);
            m.insert("warning".into(), json!(format!("앱 실행 여부 확인 실패({why})")));
            Value::Object(m)
        }
        (_, v) => v,
    }
}
fn registered_rows(c: Client) -> Value {
    let paths = default_configs(c).unwrap_or_default();
    if paths.is_empty() {
        return json!({"config": Value::Null, "note": "경로 미확정(--config 필요)"});
    }
    let one = |path: &PathBuf| match read_config(path) {
        Ok(None) => json!({"config": path, "exists": false}),
        Ok(Some(t)) => match merge(c, path, Some(&t), None) {
            Ok((_, existed)) => json!({"config": path, "exists": true, "registered": existed}),
            Err((code, _)) => json!({"config": path, "exists": true, "error": code}),
        },
        Err((code, _)) => json!({"config": path, "error": code}),
    };
    if paths.len() == 1 {
        return one(&paths[0]);
    }
    let note = "MSIX(스토어) Claude Desktop 감지: 앱이 읽는 쪽은 첫 번째(가상화) 경로로 보고되나 근거가 엇갈려 두 곳 모두 관리한다";
    json!({"msix": true, "note": note, "locations": paths.iter().map(one).collect::<Vec<_>>()})
}

#[cfg(test)]
mod msix_tests {
    use super::claude_desktop_paths;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("oh-msix-{tag}-{}-{}", std::process::id(), oh_mcp::now_stamp()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn package_folder_adds_the_virtualized_location_first() {
        let d = tmp("a");
        let (local, appdata) = (d.join("Local 사용자"), d.join("Roaming"));
        std::fs::create_dir_all(local.join("Packages/Claude_pzs8sxrjxfjjc")).unwrap();
        std::fs::create_dir_all(local.join("Packages/Other_abc")).unwrap();
        std::fs::create_dir_all(local.join("Packages/NotClaude_x")).unwrap();
        std::fs::write(local.join("Packages/Claude_file"), b"x").unwrap(); // a file, not a package folder
        let got = claude_desktop_paths(&local, &appdata);
        assert_eq!(got, vec![
            local.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude/claude_desktop_config.json"),
            appdata.join("Claude/claude_desktop_config.json"),
        ]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn claude_data_folder_is_listed_first() {
        let d = tmp("c");
        let (local, appdata) = (d.join("Local"), d.join("Roaming"));
        std::fs::create_dir_all(local.join("Packages/Claude_pzs8sxrjxfjjc")).unwrap();
        std::fs::create_dir_all(local.join("Claude-Data/claude_pzs8sxrjxfjjc")).unwrap();
        std::fs::write(local.join("Claude-Data/README.txt"), b"x").unwrap(); // a file, not a data folder
        assert_eq!(claude_desktop_paths(&local, &appdata), vec![
            local.join("Claude-Data/claude_pzs8sxrjxfjjc/claude_desktop_config.json"),
            local.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude/claude_desktop_config.json"),
            appdata.join("Claude/claude_desktop_config.json"),
        ]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn no_package_folder_keeps_only_the_plain_appdata_location() {
        let d = tmp("b");
        let (local, appdata) = (d.join("Local"), d.join("Roaming"));
        assert_eq!(claude_desktop_paths(&local, &appdata), vec![appdata.join("Claude/claude_desktop_config.json")]); // no Packages dir at all
        std::fs::create_dir_all(local.join("Packages/Other_abc")).unwrap();
        assert_eq!(claude_desktop_paths(&local, &appdata), vec![appdata.join("Claude/claude_desktop_config.json")]);
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
#[test]
fn tasklist_csv_parsing() {
    let row = "\"Claude.exe\",\"1234\",\"Console\",\"1\",\"150,000 K\"\r\n";
    assert!(tasklist_has(row, "claude.exe"));
    assert!(!tasklist_has("정보: 지정된 조건에 일치하는 작업이 실행되고 있지 않습니다.\r\n", "claude.exe"));
    assert!(!tasklist_has("INFO: No tasks are running which match the specified criteria.", "claude.exe"));
    assert!(!tasklist_has("\"claude.exe.bak\",\"1\"", "claude.exe"));
    assert!(tasklist_has("\"x.exe\",\"1\"\n\"claude.exe\",\"2\"", "CLAUDE.EXE"));
    // Claude Code vs the desktop app
    assert!(is_claude_code_path(r"C:\Users\kim\.local\bin\claude.exe"));
    assert!(is_claude_code_path(r"C:\Users\kim\.claude\local\claude.exe"));
    assert!(!is_claude_code_path(r"C:\Program Files\WindowsApps\Claude_1.0.0.0_x64__pzs8sxrjxfjjc\app\claude.exe"));
    assert!(!is_claude_code_path(r"C:\Users\kim\AppData\Local\AnthropicClaude\app-1.0.0\claude.exe"));
    assert!(!is_claude_code_path(""));
}
