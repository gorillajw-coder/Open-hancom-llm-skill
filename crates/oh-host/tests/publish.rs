//! Host publication tests (Linux/Unix). Windows lock/alias cases are run on W8 (P2).
use oh_host::{bounded_read, publish_no_clobber};
use std::fs;
use std::path::Path;
mod common;
use common::tempdir;

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    v.sort();
    v
}

#[test]
fn publishes_new_file_exactly() {
    let d = tempdir();
    let t = d.path().join("새 문서 (1).hwpx");
    publish_no_clobber(&t, b"payload").unwrap();
    assert_eq!(fs::read(&t).unwrap(), b"payload");
    assert_eq!(names(d.path()), vec!["새 문서 (1).hwpx"]);
}

#[test]
fn never_overwrites_existing_file_and_leaves_no_stage() {
    let d = tempdir();
    let t = d.path().join("out.hwpx");
    fs::write(&t, b"original").unwrap();
    assert_eq!(publish_no_clobber(&t, b"new"), Err("target_exists"));
    assert_eq!(fs::read(&t).unwrap(), b"original");
    assert_eq!(names(d.path()), vec!["out.hwpx"]);
}

#[cfg(unix)]
#[test]
fn refuses_symlink_live_or_dangling_and_hardlink_alias() {
    let d = tempdir();
    let src = d.path().join("src.hwpx");
    fs::write(&src, b"original").unwrap();
    let live = d.path().join("live.hwpx");
    std::os::unix::fs::symlink(&src, &live).unwrap();
    let dangling = d.path().join("dangling.hwpx");
    std::os::unix::fs::symlink(d.path().join("missing"), &dangling).unwrap();
    let hard = d.path().join("hard.hwpx");
    fs::hard_link(&src, &hard).unwrap();
    for t in [&live, &dangling, &hard] {
        assert_eq!(publish_no_clobber(t, b"new"), Err("target_exists"), "{t:?}");
    }
    assert!(live.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(dangling.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(!d.path().join("missing").exists());
    assert_eq!(fs::read(&src).unwrap(), b"original");
    assert_eq!(names(d.path()), vec!["dangling.hwpx", "hard.hwpx", "live.hwpx", "src.hwpx"]);
}

#[test]
fn missing_parent_is_output_io_error() {
    let d = tempdir();
    let t = d.path().join("no-such-dir").join("out.hwpx");
    assert_eq!(publish_no_clobber(&t, b"x"), Err("output_io_error"));
    assert_eq!(names(d.path()), Vec::<String>::new());
}

#[test]
fn existing_directory_is_refused() {
    let d = tempdir();
    let t = d.path().join("dir.hwpx");
    fs::create_dir(&t).unwrap();
    assert!(publish_no_clobber(&t, b"x").is_err());
    assert!(t.is_dir());
    assert_eq!(names(d.path()), vec!["dir.hwpx"]);
}

#[cfg(unix)]
#[test]
fn unwritable_directory_is_output_io_error_without_partial_file() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempdir();
    let ro = d.path().join("ro");
    fs::create_dir(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = ro.join("probe");
    let root_can_write = fs::write(&probe, b"").is_ok(); // running as root ignores modes
    let _ = fs::remove_file(&probe);
    if !root_can_write {
        assert_eq!(publish_no_clobber(&ro.join("out.hwpx"), b"x"), Err("output_io_error"));
        assert_eq!(names(&ro), Vec::<String>::new());
    }
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn concurrent_publishers_exactly_one_wins() {
    let d = tempdir();
    let t = d.path().join("race.hwpx");
    let results: Vec<_> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..16u8).map(|i| { let t = t.clone(); s.spawn(move || (i, publish_no_clobber(&t, &[i; 4096]))) }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let winners: Vec<_> = results.iter().filter(|(_, r)| r.is_ok()).collect();
    assert_eq!(winners.len(), 1);
    assert!(results.iter().filter(|(_, r)| r.is_err()).all(|(_, r)| *r == Err("target_exists")));
    assert_eq!(fs::read(&t).unwrap(), vec![winners[0].0; 4096]);
    assert_eq!(names(d.path()), vec!["race.hwpx"], "no stage files left");
}

#[test]
fn bounded_read_limits() {
    let d = tempdir();
    let f = d.path().join("in.bin");
    fs::write(&f, vec![7u8; 10]).unwrap();
    assert_eq!(bounded_read(&f, 10).unwrap().len(), 10);
    assert_eq!(bounded_read(&f, 9), Err("input_too_large"));
    assert_eq!(bounded_read(&d.path().join("missing"), 10), Err("input_io_error"));
    assert_eq!(bounded_read(d.path(), 10).err().is_some(), true);
}

#[cfg(windows)]
#[test]
fn windows_locked_existing_target_is_target_locked() {
    use std::os::windows::fs::OpenOptionsExt;
    let d = tempdir();
    let t = d.path().join("열린 문서.hwpx");
    fs::write(&t, b"original").unwrap();
    let held = fs::OpenOptions::new().read(true).share_mode(0).open(&t).unwrap();
    assert_eq!(publish_no_clobber(&t, b"new"), Err("target_locked"));
    drop(held);
    assert_eq!(publish_no_clobber(&t, b"new"), Err("target_exists"));
    assert_eq!(fs::read(&t).unwrap(), b"original");
    assert_eq!(names(d.path()), vec!["열린 문서.hwpx"]);
}

#[cfg(windows)]
#[test]
fn windows_long_unicode_path_publishes() {
    let d = tempdir();
    let mut dir = d.path().to_path_buf();
    for i in 0..6 { dir = dir.join(format!("긴 폴더 이름 {i} ________________________________________")); }
    let t = dir.join("결과 문서.hwpx");
    match fs::create_dir_all(&dir) {
        Ok(()) => { publish_no_clobber(&t, b"x").unwrap(); assert_eq!(fs::read(&t).unwrap(), b"x"); }
        Err(e) => eprintln!("long path not creatable on this host ({e}); skipped"),
    }
}

#[cfg(unix)]
#[test]
fn output_mode_matches_legacy_0600() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempdir();
    let t = d.path().join("m.hwpx");
    publish_no_clobber(&t, b"x").unwrap();
    assert_eq!(fs::metadata(&t).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(windows)]
#[test]
fn windows_readonly_or_shared_open_existing_target_is_target_exists() {
    let d = tempdir();
    let ro = d.path().join("ro.hwpx");
    fs::write(&ro, b"original").unwrap();
    let mut perm = fs::metadata(&ro).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&ro, perm.clone()).unwrap();
    assert_eq!(publish_no_clobber(&ro, b"new"), Err("target_exists"));
    // a reader that shares read/write (e.g. a viewer) is not a lock
    let shared = d.path().join("shared.hwpx");
    fs::write(&shared, b"original").unwrap();
    let _reader = fs::File::open(&shared).unwrap();
    assert_eq!(publish_no_clobber(&shared, b"new"), Err("target_exists"));
    assert_eq!(fs::read(&ro).unwrap(), b"original");
    perm.set_readonly(false);
    fs::set_permissions(&ro, perm).unwrap();
}

#[test]
fn read_error_code_separates_locked_sources_on_windows_only() {
    use oh_host::read_error_code;
    assert_eq!(read_error_code(Some(32), true), "source_locked");
    assert_eq!(read_error_code(Some(33), true), "source_locked");
    assert_eq!(read_error_code(Some(2), true), "input_io_error");
    assert_eq!(read_error_code(Some(5), true), "input_io_error");
    assert_eq!(read_error_code(None, true), "input_io_error");
    // Linux errno 32 is EPIPE and 33 EDOM, never a sharing violation.
    assert_eq!(read_error_code(Some(32), false), "input_io_error");
    assert_eq!(read_error_code(Some(33), false), "input_io_error");
}

#[test]
fn missing_source_stays_input_io_error() {
    let d = tempdir();
    assert_eq!(bounded_read(&d.path().join("nope.hwpx"), 10), Err("input_io_error"));
}
