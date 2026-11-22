//! Embeds the newest effects list in `catalogs/` (made with `--export-catalog`) and the newest Focus mode tones in `focus/` (made with
//! `--export-focus`) into the build. Each generated file says which version it came from; with none the build still works.

use std::path::{Path, PathBuf};

include!("src/catalog/version.rs");

fn newest(dir: &Path, file: &str) -> Option<(String, PathBuf)> {
    let mut found: Vec<(Vec<u64>, String, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let pack = e.path().join(file);
            let name = e.file_name().to_string_lossy().into_owned();
            (pack.is_file() && is_version(&name)).then(|| (version_key(&name), name, pack))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, name, pack)| (name, pack))
}

/// Write `<out_name>` into OUT_DIR: `pub const BUILTIN: Option<(&str, &[u8])>` with the newest `<folder>/<version>/<file>`.
fn embed(manifest: &Path, folder: &str, file: &str, out_name: &str) {
    let dir = manifest.join("..").join(folder);
    println!("cargo:rerun-if-changed=../{folder}");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join(out_name);
    let text = match newest(&dir, file) {
        Some((version, pack)) => {
            // Rerun when the chosen file changes too (a change inside a folder does not touch the folder itself).
            println!("cargo:rerun-if-changed={}", pack.display());
            let pack = pack.canonicalize().unwrap_or(pack);
            format!("Some(({version:?}, include_bytes!({:?})))", pack.to_string_lossy())
        }
        None => "None".to_owned(),
    };
    std::fs::write(out, format!("pub const BUILTIN: Option<(&str, &[u8])> = {text};\n"))
        .unwrap_or_else(|e| panic!("write {out_name}: {e}"));
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    embed(&manifest, "catalogs", "catalog.json.gz", "builtin_catalog.rs");
    embed(&manifest, "focus", "focus.json.gz", "builtin_focus.rs");
}
