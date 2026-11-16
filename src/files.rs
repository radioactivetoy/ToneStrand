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
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|c| !c.exists())
        .expect("an unused name exists")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tonestrand-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
