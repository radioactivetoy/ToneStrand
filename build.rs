//! Packs the model photos in assets/models/icons into the program, so the
//! app works wherever it is copied.

use std::fmt::Write as _;

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/models/icons");
    println!("cargo:rerun-if-changed=assets/models");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.to_ascii_lowercase().ends_with(".png")).collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut code = String::from("/// (file name, PNG bytes) of every model photo.\npub static PHOTOS: &[(&str, &[u8])] = &[\n");
    for n in &names {
        let path = dir.join(n);
        let _ = writeln!(code, "    ({n:?}, include_bytes!({:?})),", path.to_string_lossy());
    }
    code.push_str("];\n");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("photos.rs");
    std::fs::write(out, code).unwrap();
}
