//! OS layer for Open-hancom: the only crate that touches the filesystem.
//!
//! * `bounded_read`: read a regular file with an explicit size cap.
//! * `publish_no_clobber`: write bytes to a sibling temporary file, flush it to
//!   disk and atomically move it to the destination without ever replacing an
//!   existing entry (file, directory, symlink or hardlink alias).
//!   std only (no tempfile/getrandom/windows-sys):
//!   - stage: same-directory `.oh-stage-<pid>-<n>-<nanos>.tmp` via `create_new`,
//!     write, `sync_all` (Windows: `FlushFileBuffers`).
//!   - A killed publisher can leave its stage file; the next publish in the same
//!     folder deletes `.oh-stage-*.tmp` files older than 24 h.
//!   - Unix: `hard_link(stage, target)` (link(2) never replaces) → remove stage.
//!   - Windows: `MoveFileExW(stage, target, MOVEFILE_WRITE_THROUGH)` without
//!     `MOVEFILE_REPLACE_EXISTING` (fails if target exists; works on volumes
//!     without hard-link support).
//!     An existing destination held open without sharing by another process
//!     (e.g. Hancom) is reported as `target_locked` instead of `target_exists`.
//!
//! Error codes are the legacy CLI codes (`input_io_error`, `input_too_large`,
//! `output_io_error`, `target_exists`) plus `target_locked` (existing target held open) and
//! `source_locked` (input held open without read sharing); both Windows only.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub type Result<T> = std::result::Result<T, &'static str>;

/// Create a directory and its parents (no-op when it exists).
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|_| "workdir_unavailable")
}

/// Read at most `max` bytes from `path`; larger files fail with `input_too_large`.
/// Symlinked inputs are followed (legacy behaviour); the file is only read.
pub fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>> {
    let f = File::open(path).map_err(|e| read_error_code(e.raw_os_error(), cfg!(windows)))?;
    if f.metadata().map_err(|_| "input_io_error")?.len() > max {
        return Err("input_too_large");
    }
    let mut raw = Vec::new();
    f.take(max + 1).read_to_end(&mut raw).map_err(|e| read_error_code(e.raw_os_error(), cfg!(windows)))?;
    if raw.len() as u64 > max {
        return Err("input_too_large");
    }
    Ok(raw)
}

/// Error code for a failed open/read. Windows ERROR_SHARING_VIOLATION (32) and
/// ERROR_LOCK_VIOLATION (33) mean another process holds the source without read
/// sharing: `source_locked`. Everything else stays `input_io_error`.
/// `windows` is a parameter so the mapping is testable on every OS.
pub fn read_error_code(raw_os_error: Option<i32>, windows: bool) -> &'static str {
    match raw_os_error {
        Some(32) | Some(33) if windows => "source_locked",
        _ => "input_io_error",
    }
}

/// The user's Documents folder. Windows: the shell known folder (follows OneDrive
/// folder redirection), falling back to `%USERPROFILE%\Documents`. Other OS: `$HOME/Documents`.
pub fn documents_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(p) = known_documents() { return Some(p); }
        return std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Documents"));
    }
    #[cfg(not(windows))]
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents"))
}

#[cfg(windows)]
fn known_documents() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: *mut core::ffi::c_void, out: *mut *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(p: *mut core::ffi::c_void);
    }
    // FOLDERID_Documents {FDD39AD0-238F-46AF-ADB4-6C85480369C7}
    let id = Guid(0xFDD39AD0, 0x238F, 0x46AF, [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7]);
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `out` receives a CoTaskMem-allocated NUL-terminated UTF-16 string on success;
    // it is copied and freed exactly once. On failure the pointer is not used.
    unsafe {
        let hr = SHGetKnownFolderPath(&id, 0, core::ptr::null_mut(), &mut out);
        let res = if hr == 0 && !out.is_null() {
            let mut n = 0;
            while *out.add(n) != 0 { n += 1; }
            Some(PathBuf::from(std::ffi::OsString::from_wide(core::slice::from_raw_parts(out, n))))
        } else { None };
        if !out.is_null() { CoTaskMemFree(out as *mut _); }
        res
    }
}

/// Directory that receives the temporary stage file (same volume as target).
fn stage_dir(target: &Path) -> &Path {
    target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// Stage file that is removed on drop unless it was moved into place.
struct Stage {
    path: PathBuf,
    armed: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn create_stage(dir: &Path) -> Result<(Stage, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    for _ in 0..64 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".oh-stage-{}-{n}-{nanos}.tmp", std::process::id()));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600); // legacy tempfile mode
        match opts.open(&path) {
            Ok(f) => return Ok((Stage { path, armed: true }, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("output_io_error"),
        }
    }
    Err("output_io_error")
}

/// Stage files older than this are leftovers of a killed publisher (a live one
/// finishes in milliseconds) and are removed by the next publish in that folder.
const STALE_STAGE_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// `.oh-stage-<pid>-<n>-<nanos>.tmp`, the exact shape `create_stage` produces.
fn is_stage_name(name: &str) -> bool {
    let Some(mid) = name.strip_prefix(".oh-stage-").and_then(|r| r.strip_suffix(".tmp")) else { return false };
    let parts: Vec<&str> = mid.split('-').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Best effort: delete stale stage files (regular files only) next to the target.
/// Errors are ignored; a file another process still holds simply stays.
fn sweep_stale_stages(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let name = e.file_name();
        if !name.to_str().is_some_and(is_stage_name) { continue; }
        let Ok(meta) = e.metadata() else { continue };
        let old = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= STALE_STAGE_AGE);
        if meta.is_file() && old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Publish `bytes` at `target` atomically; never overwrites. On any failure no
/// partial file is left at `target` and the stage file is removed.
pub fn publish_no_clobber(target: &Path, bytes: &[u8]) -> Result<()> {
    // No pre-check: the atomic no-replace primitive is the only authority, so a
    // destination created concurrently (or an existing symlink/hardlink alias,
    // even dangling) is refused without a check-then-act race.
    sweep_stale_stages(stage_dir(target));
    let (mut stage, mut file) = create_stage(stage_dir(target))?;
    file.write_all(bytes).map_err(|_| "output_io_error")?;
    file.sync_all().map_err(|_| "output_io_error")?;
    drop(file); // Windows cannot move an open file
    commit(&mut stage, target)
}

#[cfg(not(windows))]
fn commit(stage: &mut Stage, target: &Path) -> Result<()> {
    match std::fs::hard_link(&stage.path, target) {
        Ok(()) => {
            // target now names the data; the stage name is removed by Drop.
            if let Some(dir) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
                if let Ok(d) = File::open(dir) { let _ = d.sync_all(); }
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(existing_target_code(target)),
        Err(_) => Err("output_io_error"),
    }
}

/// NUL-terminated UTF-16 Windows extended-length form (`\\?\C:\...`, `\\?\UNC\server\...`) so the
/// raw MoveFileExW call is not limited to MAX_PATH (std already does this for
/// its own file APIs).
#[cfg(windows)]
fn verbatim_wide(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let w: Vec<u16> = abs.as_os_str().encode_wide().collect();
    let bs = b'\\' as u16;
    let lit = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let mut out = if w.starts_with(&lit(r"\\?\")) || w.starts_with(&lit(r"\\.\")) {
        w
    } else if w.len() >= 2 && w[0] == bs && w[1] == bs {
        let mut o = lit(r"\\?\UNC\");
        o.extend_from_slice(&w[2..]);
        o
    } else {
        let mut o = lit(r"\\?\");
        o.extend_from_slice(&w);
        o
    };
    out.push(0);
    out
}

#[cfg(windows)]
fn commit(stage: &mut Stage, target: &Path) -> Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let (from, to) = (verbatim_wide(&stage.path), verbatim_wide(target));
    // SAFETY: both buffers are NUL-terminated UTF-16 and live across the call.
    let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok != 0 {
        stage.armed = false;
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    // ERROR_FILE_EXISTS (80) / ERROR_ALREADY_EXISTS (183), or any failure while
    // the destination exists (e.g. ACCESS_DENIED during a concurrent commit).
    if matches!(e.raw_os_error(), Some(80) | Some(183)) || target.symlink_metadata().is_ok() {
        return Err(existing_target_code(target));
    }
    match e.raw_os_error() {
        Some(32) | Some(33) => Err("target_locked"),
        _ => Err("output_io_error"),
    }
}

#[cfg(not(windows))]
fn existing_target_code(_target: &Path) -> &'static str {
    "target_exists"
}

/// Windows: an existing destination that another process holds open without
/// sharing (Hancom keeps opened documents locked) is `target_locked`.
#[cfg(windows)]
fn existing_target_code(target: &Path) -> &'static str {
    use std::os::windows::fs::OpenOptionsExt;
    // Probe with write access but full sharing (read|write|delete), never
    // truncating or writing: only a holder that denies write sharing (an editor
    // that opened the document) makes this a sharing violation. Concurrent
    // probes from other publishers share everything, so they never collide.
    const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    match OpenOptions::new().write(true).share_mode(SHARE_ALL).open(target) {
        Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => "target_locked",
        _ => "target_exists",
    }
}

/// Write `bytes` at `target`, replacing an existing file atomically (stage + rename). For the
/// host's own records (project facts), never for user documents (those use `publish_no_clobber`).
pub fn replace_file(target: &Path, bytes: &[u8]) -> Result<()> {
    sweep_stale_stages(stage_dir(target));
    let (mut stage, mut file) = create_stage(stage_dir(target))?;
    file.write_all(bytes).map_err(|_| "output_io_error")?;
    file.sync_all().map_err(|_| "output_io_error")?;
    drop(file);
    match std::fs::rename(&stage.path, target) {
        Ok(()) => {
            stage.armed = false;
            Ok(())
        }
        Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => Err("target_locked"),
        Err(_) => Err("output_io_error"),
    }
}

/// Entries of a directory, sorted by name: (name, is_dir). Symlinks are reported as files and
/// never followed into. Names that are not valid UTF-8 fail (`non_utf8_path`).
pub fn list_dir(path: &Path) -> Result<Vec<(String, bool)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(path).map_err(|e| read_error_code(e.raw_os_error(), cfg!(windows)))? {
        let e = e.map_err(|_| "input_io_error")?;
        let name = e.file_name().into_string().map_err(|_| "non_utf8_path")?;
        let dir = e.file_type().map_err(|_| "input_io_error")?.is_dir();
        out.push((name, dir));
    }
    out.sort();
    Ok(out)
}
