//! DESIGN §11.2 Q-series (host/CLI level): permission, lock, path, environment
//! and interruption cases, driven through the real `oh` binary. Windows-only
//! cases use child processes (PowerShell) as the "other process" that holds a file.
//! Expected values are written by hand, not computed with the code under test.
mod common;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SYNTH: &[u8] = include_bytes!("../../oh-document/tests/fixtures/synthetic.hwpx");
const CREATE: &str = include_str!("../../oh-document/tests/fixtures/create.json");
const MEDIA_CREATE: &str = include_str!("../../oh-document/tests/fixtures/media-create.json");

fn run(cmd: &mut Command) -> (i32, Value) {
    let out = cmd.output().unwrap();
    assert!(out.stderr.is_empty(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap(), serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("stdout not JSON ({e}): {:?}", out.stdout)))
}
fn oh(args: &[&dyn AsRef<std::ffi::OsStr>]) -> (i32, Value) {
    run(Command::new(env!("CARGO_BIN_EXE_oh")).args(args.iter().map(|a| a.as_ref())))
}
fn failed(code: &str) -> Value { json!({"error": code, "written": false}) }
fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<_> = fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    v.sort();
    v
}
/// source + noop-edit plan in `d`; returns (source, plan).
fn fixture(d: &Path) -> (PathBuf, PathBuf) {
    let src = d.join("input.hwpx");
    fs::write(&src, SYNTH).unwrap();
    let rev = oh(&[&"inspect", &src]).1["revision"].as_str().unwrap().to_string();
    let plan = d.join("plan.json");
    fs::write(&plan, json!({"revision": rev, "operations": [{"op": "replace_text", "target": "Contents/section0.xml#t2", "expected": "항목", "text": "항목"}]}).to_string()).unwrap();
    (src, plan)
}
fn apply(src: &Path, plan: &Path, out: &Path) -> (i32, Value) {
    oh(&[&"apply", &src, &"--plan", &plan, &"--output", &out])
}

// ---------- Q01: output/work folder not writable ----------
#[test]
fn q01_missing_or_unwritable_output_folder_is_output_io_error() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    assert_eq!(apply(&src, &plan, &d.path().join("nodir").join("o.hwpx")), (2, failed("output_io_error")));
    let ro = d.path().join("ro");
    fs::create_dir(&ro).unwrap();
    let guard = deny_writes(&ro);
    let probe = fs::write(ro.join("probe"), b"x"); // running as root / admin may ignore the denial
    if probe.is_ok() {
        let _ = fs::remove_file(ro.join("probe"));
        drop(guard);
        eprintln!("q01: write denial not effective for this account; skipped the unwritable-dir assertion");
        return;
    }
    assert_eq!(apply(&src, &plan, &ro.join("o.hwpx")), (2, failed("output_io_error")));
    drop(guard);
    assert_eq!(names(&ro), Vec::<String>::new(), "no partial output, no stage file");
}

struct Undo(Option<Box<dyn FnOnce()>>);
impl Drop for Undo { fn drop(&mut self) { if let Some(f) = self.0.take() { f(); } } }
#[cfg(unix)]
fn deny_writes(dir: &Path) -> Undo {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o500)).unwrap();
    let d = dir.to_path_buf();
    Undo(Some(Box::new(move || { let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o700)); })))
}
#[cfg(windows)]
fn deny_writes(dir: &Path) -> Undo {
    // Deny create-file/create-subdirectory (WD,AD) to Everyone, inherited.
    let st = Command::new("icacls").arg(dir).args(["/deny", "*S-1-1-0:(OI)(CI)(WD,AD)"]).stdout(Stdio::null()).status().unwrap();
    assert!(st.success(), "icacls deny failed");
    let d = dir.to_path_buf();
    Undo(Some(Box::new(move || { let _ = Command::new("icacls").arg(&d).args(["/remove:d", "*S-1-1-0"]).stdout(Stdio::null()).status(); })))
}

// ---------- Q02 / Q05: another process holds a file ----------
/// Child PowerShell that opens `path` with the given access/share and sleeps
/// until killed. Returns once the file is really open.
#[cfg(windows)]
struct Holder(std::process::Child);
#[cfg(windows)]
impl Drop for Holder { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
#[cfg(windows)]
fn hold(path: &Path, access: &str, share: &str) -> Holder {
    let ready = common::tempdir();
    let flag = ready.path().join("ready");
    let script = format!(
        "$f=[IO.File]::Open('{}','Open','{access}','{share}'); [IO.File]::WriteAllText('{}','1'); Start-Sleep 120",
        path.display().to_string().replace('\'', "''"), flag.display().to_string().replace('\'', "''"));
    let child = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let mut h = Holder(child);
    for _ in 0..200 {
        if flag.exists() { return h; }
        if let Some(st) = h.0.try_wait().unwrap() { panic!("holder exited early: {st}"); }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("holder never opened {}", path.display());
}

#[cfg(windows)]
#[test]
fn q02_output_held_by_other_process_without_write_sharing_is_target_locked() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let dst = d.path().join("out.hwpx");
    fs::write(&dst, b"existing").unwrap();
    // (access, share) combinations of a document viewer/editor that deny writers
    for (access, share) in [("Read", "None"), ("Read", "Read"), ("ReadWrite", "None"), ("ReadWrite", "Read")] {
        let h = hold(&dst, access, share);
        assert_eq!(apply(&src, &plan, &dst), (2, failed("target_locked")), "{access}/{share}");
        drop(h);
        assert_eq!(fs::read(&dst).unwrap(), b"existing");
    }
    // A reader that shares everything does not lock; plain target_exists.
    for (access, share) in [("Read", "ReadWrite"), ("Read", "ReadWrite, Delete")] {
        let h = hold(&dst, access, share);
        assert_eq!(apply(&src, &plan, &dst), (2, failed("target_exists")), "{access}/{share}");
        drop(h);
    }
    assert_eq!(fs::read(&dst).unwrap(), b"existing");
    assert_eq!(fs::read(&src).unwrap(), SYNTH);
    assert_eq!(names(d.path()), vec!["input.hwpx", "out.hwpx", "plan.json"], "no stage leftovers");
}

#[cfg(windows)]
#[test]
fn q05_source_held_by_other_process_is_readable_when_sharing_reads() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    for (access, share) in [("Read", "Read"), ("Read", "ReadWrite"), ("ReadWrite", "ReadWrite")] {
        let h = hold(&src, access, share);
        let (rc, info) = oh(&[&"inspect", &src]);
        assert_eq!(rc, 0, "{access}/{share}: {info}");
        let dst = d.path().join(format!("o-{access}-{share}.hwpx").replace(['/', ' ', ','], "_"));
        assert_eq!(apply(&src, &plan, &dst).0, 0, "{access}/{share}");
        drop(h);
    }
    assert_eq!(fs::read(&src).unwrap(), SYNTH);
}

#[cfg(windows)]
#[test]
fn q05_source_held_exclusively_is_source_locked_and_untouched() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let h = hold(&src, "Read", "None");
    let dst = d.path().join("o.hwpx");
    assert_eq!(apply(&src, &plan, &dst), (2, failed("source_locked")));
    assert_eq!(oh(&[&"inspect", &src]), (2, failed("source_locked")));
    drop(h);
    assert_eq!(fs::read(&src).unwrap(), SYNTH);
    assert_eq!(names(d.path()), vec!["input.hwpx", "plan.json"]);
}

#[test]
fn q05_read_only_source_is_readable_and_unchanged() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let mut perm = fs::metadata(&src).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&src, perm).unwrap();
    let before = fs::metadata(&src).unwrap().modified().unwrap();
    assert_eq!(oh(&[&"inspect", &src]).0, 0);
    let dst = d.path().join("o.hwpx");
    assert_eq!(apply(&src, &plan, &dst).0, 0);
    assert_eq!(fs::read(&dst).unwrap(), SYNTH);
    assert_eq!(fs::read(&src).unwrap(), SYNTH);
    assert_eq!(fs::metadata(&src).unwrap().modified().unwrap(), before, "source mtime unchanged");
    assert!(fs::metadata(&src).unwrap().permissions().readonly(), "still read-only");
    let mut perm = fs::metadata(&src).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    fs::set_permissions(&src, perm).unwrap(); // let TempDir clean up
}

// ---------- Q03 ----------
#[test]
fn q03_existing_output_is_target_exists_and_untouched() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let dst = d.path().join("out.hwpx");
    fs::write(&dst, b"existing").unwrap();
    assert_eq!(apply(&src, &plan, &dst), (2, failed("target_exists")));
    assert_eq!(fs::read(&dst).unwrap(), b"existing");
    let dir_target = d.path().join("adir.hwpx");
    fs::create_dir(&dir_target).unwrap();
    assert_eq!(apply(&src, &plan, &dir_target), (2, failed("target_exists")));
    assert_eq!(names(d.path()), vec!["adir.hwpx", "input.hwpx", "out.hwpx", "plan.json"]);
}

// ---------- Q04: Korean / space / long paths ----------
#[test]
fn q04_korean_space_and_over_260_char_paths() {
    let d = common::tempdir();
    // total path length well above 260 chars, built from components <= 100 chars
    let mut dir = d.path().join("한글 폴더 이름 with spaces");
    for i in 0..4 {
        dir = dir.join(format!("{}-{}", "가나다라마바사 ".repeat(8).trim_end(), i)); // 8*8-1 = 63 chars
    }
    fs::create_dir_all(&dir).unwrap();
    assert!(dir.as_os_str().len() > 260, "path only {} chars", dir.as_os_str().len());
    let (src, plan) = fixture(&dir);
    let dst = dir.join("결재 서류 (최종) #1 – 사본.hwpx");
    let (rc, r) = apply(&src, &plan, &dst);
    assert_eq!(rc, 0, "{r}");
    assert_eq!(fs::read(&dst).unwrap(), SYNTH);
    assert_eq!(r["path"], json!(dst));
    assert_eq!(apply(&src, &plan, &dst), (2, failed("target_exists")));
    assert_eq!(oh(&[&"validate", &dst]).1["valid_supported_structure"], json!(true));
    assert_eq!(names(&dir), vec!["input.hwpx", "plan.json", "결재 서류 (최종) #1 – 사본.hwpx"]);
}

// ---------- Q07: binary moved elsewhere, empty environment ----------
#[test]
fn q07_copied_binary_with_empty_environment_behaves_identically() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let moved = d.path().join("elsewhere 폴더");
    fs::create_dir(&moved).unwrap();
    let exe = moved.join(Path::new(env!("CARGO_BIN_EXE_oh")).file_name().unwrap());
    fs::copy(env!("CARGO_BIN_EXE_oh"), &exe).unwrap();
    let cwd = common::tempdir();
    let normal = d.path().join("normal.hwpx");
    assert_eq!(apply(&src, &plan, &normal).0, 0);
    let isolated = d.path().join("isolated.hwpx");
    let (rc, r) = run(Command::new(&exe).env_clear().current_dir(cwd.path())
        .args(["apply".as_ref(), src.as_os_str(), "--plan".as_ref(), plan.as_os_str(), "--output".as_ref(), isolated.as_os_str()]));
    assert_eq!(rc, 0, "{r}");
    assert_eq!(fs::read(&isolated).unwrap(), fs::read(&normal).unwrap());
    let (rc, s) = run(Command::new(&exe).env_clear().current_dir(cwd.path()).arg("schema"));
    assert_eq!(rc, 0);
    assert_eq!(s, oh(&[&"schema"]).1);
}

// ---------- Q08 (CLI argument level) ----------
#[test]
fn q08_bad_arguments_are_strictly_rejected() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let out = d.path().join("o.hwpx");
    // duplicated / reordered / missing flags
    assert_eq!(oh(&[&"apply", &src, &"--plan", &plan, &"--plan", &plan, &"--output", &out]), (2, failed("unsupported_command")));
    assert_eq!(oh(&[&"apply", &src, &"--output", &out, &"--plan", &plan]), (2, failed("unsupported_command")));
    assert_eq!(oh(&[&"apply", &src, &"--plan", &plan]), (2, failed("unsupported_command")));
    assert_eq!(oh(&[&""]), (2, failed("unsupported_command")));
    assert_eq!(oh(&[&"inspect", &""]), (2, failed("input_io_error")));
    // very long path argument (no panic, clean error)
    let long = "x".repeat(20_000);
    assert_eq!(oh(&[&"inspect", &long]), (2, failed("input_io_error")));
    // NUL-free but unpaired-surrogate / invalid-UTF-8 argument
    #[cfg(unix)]
    let bad = { use std::os::unix::ffi::OsStringExt; std::ffi::OsString::from_vec(vec![b'a', 0xff, 0xfe]) };
    #[cfg(windows)]
    let bad = { use std::os::windows::ffi::OsStringExt; std::ffi::OsString::from_wide(&[0x61, 0xD800]) };
    assert_eq!(run(Command::new(env!("CARGO_BIN_EXE_oh")).arg("inspect").arg(&bad)), (2, failed("non_utf8_argument")));
    // same input, repeated calls: identical results, nothing written
    for _ in 0..3 { assert_eq!(oh(&[&"inspect", &src]).0, 0); }
    assert_eq!(names(d.path()), vec!["input.hwpx", "plan.json"]);
}

// ---------- Q10: disk full / interruption ----------
#[cfg(unix)]
#[test]
fn q10_write_failure_leaves_no_target_and_no_stage() {
    // RLIMIT_FSIZE of 512 bytes makes the stage write fail with EFBIG (SIGXFSZ ignored).
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let out = d.path().join("o.hwpx");
    let o = Command::new("sh")
        .args(["-c", "trap '' XFSZ; ulimit -f 1; exec \"$0\" apply \"$1\" --plan \"$2\" --output \"$3\"", env!("CARGO_BIN_EXE_oh")])
        .args([&src, &plan, &out]).output().unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(serde_json::from_slice::<Value>(&o.stdout).unwrap(), failed("output_io_error"));
    assert_eq!(names(d.path()), vec!["input.hwpx", "plan.json"]);
}

/// Kill `oh create` at many different moments: the target is either absent or a
/// complete document, never partial. Counts leftover stage files (informational).
#[test]
fn q10_kill_at_random_moments_never_leaves_partial_target() {
    let d = common::tempdir();
    let spec = d.path().join("spec.json");
    fs::write(&spec, MEDIA_CREATE).unwrap();
    let (mut absent, mut present, mut stages) = (0, 0, 0);
    for i in 0..120u64 {
        let out = d.path().join(format!("o{i}.hwpx"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_oh"))
            .args(["create".as_ref(), "--spec".as_ref(), spec.as_os_str(), "--output".as_ref(), out.as_os_str()])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_micros((i % 40) * 250));
        let _ = child.kill();
        let _ = child.wait();
        if out.exists() {
            present += 1;
            let (rc, v) = oh(&[&"validate", &out]);
            assert_eq!((rc, &v["valid_supported_structure"]), (0, &json!(true)), "partial/invalid target after kill #{i}");
        } else {
            absent += 1;
        }
    }
    for n in names(d.path()) {
        if n.starts_with(".oh-stage-") { stages += 1; }
    }
    eprintln!("q10 kill loop: target present={present} absent={absent} leftover stage files={stages}");
}

/// Stale stage files from killed runs are cleaned by the next publish; fresh
/// ones (a concurrent publisher) and unrelated files are left alone.
#[test]
fn q10_stale_stage_files_are_cleaned_on_next_publish() {
    let d = common::tempdir();
    let (src, plan) = fixture(d.path());
    let old = d.path().join(".oh-stage-1-0-0.tmp");
    let fresh = d.path().join(".oh-stage-2-0-0.tmp");
    let other = d.path().join(".oh-stage-note.txt");
    let user = d.path().join("user.tmp");
    for p in [&old, &fresh, &other, &user] { fs::write(p, b"x").unwrap(); }
    let two_days = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 3600);
    for p in [&old, &other] {
        fs::OpenOptions::new().write(true).open(p).unwrap().set_modified(two_days).unwrap();
    }
    assert_eq!(apply(&src, &plan, &d.path().join("o.hwpx")).0, 0);
    assert_eq!(names(d.path()), vec![".oh-stage-2-0-0.tmp", ".oh-stage-note.txt", "input.hwpx", "o.hwpx", "plan.json", "user.tmp"]);
}

#[test]
fn create_smoke_for_fixture_sanity() {
    let d = common::tempdir();
    let spec = d.path().join("c.json");
    fs::write(&spec, CREATE).unwrap();
    assert_eq!(oh(&[&"create", &"--spec", &spec, &"--output", &d.path().join("c.hwpx")]).0, 0);
}
