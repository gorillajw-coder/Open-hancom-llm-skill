//! Portable scratch directory for tests: std only, under std::env::temp_dir().
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempDir(PathBuf);
impl TempDir {
    pub fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; let _ = fix_perms(&self.0, |p| { let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)); }); }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[cfg(unix)]
fn fix_perms(dir: &Path, f: impl Fn(&Path) + Copy) -> std::io::Result<()> {
    f(dir);
    for e in std::fs::read_dir(dir)? { let e = e?; if e.file_type()?.is_dir() { fix_perms(&e.path(), f)?; } }
    Ok(())
}
pub fn tempdir() -> TempDir {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let p = std::env::temp_dir().join(format!("oh-test-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&p).unwrap();
    TempDir(p)
}
