//! Safe file writes: never leave a half-written preset behind, never write over
//! a file that is already there unless asked to.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` through a temporary file next to it, so a crash or a
/// full disk leaves either the old file or the new one, never a truncated mix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        // On Windows, std's rename replaces an existing file (MoveFileEx with REPLACE_EXISTING).
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `dir/file_name`, or `dir/stem (2).ext`, `dir/stem (3).ext`, ... when that is taken.
pub fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    let first = dir.join(file_name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(file_name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| file_name.to_owned());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).expect("an unused name exists")
}

/// Copy `from` into `dir` under its own name, picking a new name rather than replacing a file.
pub fn copy_into(from: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let name = from
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let to = unique_path(dir, &name);
    std::fs::copy(from, &to)?;
    Ok(to)
}

/// Copy every file under `from` whose relative path does not exist under `to`, creating folders;
/// never overwrites. Returns how many files were copied.
pub fn copy_missing(from: &Path, to: &Path) -> std::io::Result<usize> {
    copy_missing_skipping(from, to, &|_| false)
}

/// `copy_missing`, leaving out every path for which `skip` is true (files and folders).
pub fn copy_missing_skipping(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool) -> std::io::Result<usize> {
    let mut copied = 0;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let src = e.path();
        if skip(&src) {
            continue;
        }
        let dst = to.join(e.file_name());
        let kind = e.file_type()?;
        if kind.is_dir() {
            copied += copy_missing_skipping(&src, &dst, skip)?;
        } else if kind.is_file() && !dst.exists() {
            std::fs::create_dir_all(to)?;
            std::fs::copy(&src, &dst)?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Unix seconds as (year, month, day, hour, minute) in UTC.
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, (rem / 3600) as u32, (rem % 3600 / 60) as u32)
}

/// "2026-11-16 1403": a sortable stamp for folder names (UTC).
pub fn stamp_text(secs: u64) -> String {
    let (y, mo, d, h, mi) = civil(secs);
    format!("{y}-{mo:02}-{d:02} {h:02}{mi:02}")
}

/// Whether `delete_file` can put files in a recycle bin on this system (Windows only so
/// far). Elsewhere it deletes for good, so callers always ask first.
pub const HAS_RECYCLE_BIN: bool = cfg!(windows);

/// Remove a file the person chose to delete: to the Recycle Bin on Windows (so it can be
/// taken back), for good elsewhere. On Windows a file that can't be recycled is left alone
/// and an error returned; it is never deleted for good as a fallback.
pub fn delete_file(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        recycle(path)
    }
    #[cfg(not(windows))]
    {
        std::fs::remove_file(path)
    }
}

#[cfg(windows)]
fn recycle(path: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct ShFileOpStruct {
        hwnd: *mut core::ffi::c_void,
        func: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        aborted: i32,
        mappings: *mut core::ffi::c_void,
        title: *const u16,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOpStruct) -> i32;
    }
    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOERRORUI: u16 = 0x0400;

    // The shell only recycles a full path; a relative one would be deleted for good.
    let full = std::path::absolute(path)?;
    if !full.is_absolute() || !full.is_file() {
        return Err(Error::new(ErrorKind::NotFound, format!("{} is not a file", path.display())));
    }
    // A list of paths ends with two zero characters.
    let mut from: Vec<u16> = full.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = ShFileOpStruct {
        hwnd: core::ptr::null_mut(),
        func: FO_DELETE,
        from: from.as_ptr(),
        to: core::ptr::null(),
        flags: FOF_ALLOWUNDO | FOF_SILENT | FOF_NOERRORUI,
        aborted: 0,
        mappings: core::ptr::null_mut(),
        title: core::ptr::null(),
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.aborted != 0 || full.exists() {
        return Err(Error::other(format!("could not move {} to the Recycle Bin (code {code})", path.display())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moves a scratch file to the real Recycle Bin, so it is left out of normal runs:
    /// `cargo test -- --ignored recycles_to_the_bin`.
    #[test]
    #[ignore]
    fn recycles_to_the_bin() {
        let p = std::env::temp_dir().join(format!("tonestrand-recycle-test-{}.txt", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        delete_file(&p).unwrap();
        assert!(!p.exists());
        assert!(delete_file(&p).is_err());
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tonestrand-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copy_missing_never_overwrites() {
        let d = scratch("copy-a");
        let (from, to) = (d.join("from"), d.join("to"));
        std::fs::create_dir_all(from.join("sub")).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(from.join("a.txt"), "new").unwrap();
        std::fs::write(from.join("sub/b.txt"), "b").unwrap();
        std::fs::write(to.join("a.txt"), "old").unwrap();
        assert_eq!(copy_missing(&from, &to).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(to.join("a.txt")).unwrap(), "old");
        assert!(to.join("sub/b.txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_missing_twice_copies_nothing_more() {
        let d = scratch("copy-b");
        let (from, to) = (d.join("from"), d.join("to"));
        std::fs::create_dir_all(from.join("sub")).unwrap();
        std::fs::write(from.join("a.txt"), "a").unwrap();
        std::fs::write(from.join("sub/b.txt"), "b").unwrap();
        assert_eq!(copy_missing(&from, &to).unwrap(), 2);
        assert_eq!(copy_missing(&from, &to).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_missing_skips_what_it_is_told() {
        let d = scratch("copy-c");
        let (from, to) = (d.join("from"), d.join("to"));
        std::fs::create_dir_all(from.join("cache")).unwrap();
        std::fs::write(from.join("cache/x.txt"), "x").unwrap();
        std::fs::write(from.join("a.txt"), "a").unwrap();
        assert_eq!(copy_missing_skipping(&from, &to, &|p| p.ends_with("cache")).unwrap(), 1);
        assert!(!to.join("cache").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let d = scratch("atomic");
        let p = d.join("a.hlx");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unique_names_never_clash() {
        let d = scratch("unique");
        assert_eq!(unique_path(&d, "x.wav"), d.join("x.wav"));
        std::fs::write(d.join("x.wav"), b"").unwrap();
        assert_eq!(unique_path(&d, "x.wav"), d.join("x (2).wav"));
        std::fs::write(d.join("x (2).wav"), b"").unwrap();
        assert_eq!(unique_path(&d, "x.wav"), d.join("x (3).wav"));
        let src = d.join("x.wav");
        let sub = d.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(copy_into(&src, &sub).unwrap(), sub.join("x.wav"));
        assert_eq!(copy_into(&src, &sub).unwrap(), sub.join("x (2).wav"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
