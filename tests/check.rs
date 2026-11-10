//! End-to-end exit contract for `--check`: the gate mode lint steps rely on.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn whitespace(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whitespace"))
        .args(args)
        .arg(dir)
        .output()
        .expect("run whitespace")
}

#[test]
fn clean_tree_check_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "clean\n").unwrap();
    let out = whitespace(&["-r", "--check"], dir.path());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

#[test]
fn dirty_files_check_exits_one_with_count_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "trailing   \n").unwrap();
    fs::write(dir.path().join("b.txt"), "also \n").unwrap();
    fs::write(dir.path().join("c.txt"), "clean\n").unwrap();
    let out = whitespace(&["-r", "--check"], dir.path());
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("2 file(s) carry trailing whitespace"), "{stderr}");
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "trailing   \n");
}

#[test]
fn plain_run_with_findings_fixes_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "trailing   \n").unwrap();
    let out = whitespace(&["-r"], dir.path());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "trailing\n");
}

fn assert_scan_failure(out: &Output, dirs: usize, files: usize) {
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let want = format!("{dirs} director(ies) and {files} file(s) failed");
    assert!(stderr.contains(&want), "want {want:?} in {stderr}");
}

#[test]
fn missing_directory_check_fails() {
    let dir = tempfile::tempdir().unwrap();
    let out = whitespace(&["-r", "--check"], &dir.path().join("nope"));
    assert_scan_failure(&out, 1, 0);
}

#[test]
fn every_failed_directory_is_counted() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.txt");
    fs::write(&file, "clean\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_whitespace"))
        .args(["-r", "--check"])
        .arg(dir.path().join("nope"))
        .arg(&file)
        .output()
        .expect("run whitespace");
    assert_scan_failure(&out, 2, 0);
}

#[test]
fn missing_directory_plain_run_still_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let out = whitespace(&["-r"], &dir.path().join("nope"));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

#[test]
fn binary_file_is_skipped_not_failed() {
    // Not a default binary extension, so the processor's NUL-byte skip is under test.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("blob.raw"), b"\x00\x01binary  \n").unwrap();
    let out = whitespace(&["-r", "--check"], dir.path());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

#[cfg(unix)]
#[test]
fn unreadable_file_check_fails() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("locked.txt");
    fs::write(&file, "clean\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&file).is_ok() {
        return; // running as root: permissions cannot make the file unreadable
    }
    let out = whitespace(&["-r", "--check"], dir.path());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert_scan_failure(&out, 0, 1);
}

#[cfg(unix)]
#[test]
fn unreadable_files_and_subdirectory_are_all_counted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let sub = dir.path().join("sub");
    fs::write(&a, "clean\n").unwrap();
    fs::write(&b, "clean\n").unwrap();
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("c.txt"), "clean\n").unwrap();
    for p in [&a, &b, &sub] {
        fs::set_permissions(p, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let readable = fs::read(&a).is_ok();
    let out = whitespace(&["-r", "--check"], dir.path());
    fs::set_permissions(&a, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&b, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return; // running as root: permissions cannot make these unreadable
    }
    assert_scan_failure(&out, 0, 3);
}

#[cfg(unix)]
#[test]
fn unreadable_directory_under_an_excluded_path_is_not_counted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "clean\n").unwrap();
    let locked = dir.path().join("node_modules").join("pkg").join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("c.txt"), "clean\n").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&locked).is_ok();
    let out = whitespace(&["-r", "--check"], dir.path());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return; // running as root: permissions cannot make the directory unreadable
    }
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}
