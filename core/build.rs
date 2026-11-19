//! Embeds the newest effects list in `catalogs/` (made with `--export-catalog`) into the build.
//! The generated file says which HX Edit version it came from; with no list the build still works.

use std::path::{Path, PathBuf};

include!("src/catalog/version.rs");

fn newest(dir: &Path) -> Option<(String, PathBuf)> {
    let mut found: Vec<(Vec<u64>, String, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let pack = e.path().join("catalog.json.gz");
            let name = e.file_name().to_string_lossy().into_owned();
            (pack.is_file() && is_version(&name)).then(|| (version_key(&name), name, pack))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, name, pack)| (name, pack))
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let catalogs = manifest.join("..").join("catalogs");
    println!("cargo:rerun-if-changed=../catalogs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("builtin_catalog.rs");
    let text = match newest(&catalogs) {
        Some((version, pack)) => {
            // Rerun when the chosen file changes too (a change inside a folder does not touch `catalogs/` itself).
            println!("cargo:rerun-if-changed={}", pack.display());
            let pack = pack.canonicalize().unwrap_or(pack);
            format!("Some(({version:?}, include_bytes!({:?})))", pack.to_string_lossy())
        }
        None => "None".to_owned(),
    };
    std::fs::write(out, format!("pub const BUILTIN: Option<(&str, &[u8])> = {text};\n")).expect("write builtin_catalog.rs");
}
