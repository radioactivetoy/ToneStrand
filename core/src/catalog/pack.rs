//! The catalog saved as a compact pack and read back identically, for a device that can't read HX Edit's files (the Android app).
//!
//! A pack is gzip of one JSON document: `{ "format", "sym", "models", "category", "groups" }`. The symbol index and the NAM block's
//! look are not stored; `Catalog::assemble` works them out for both loading routes.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use super::{Catalog, CatalogSource, ModelInfo, PickGroup, Rgb, SymEntry};

/// The pack layout this build writes and reads.
pub const PACK_FORMAT: u32 = 1;

/// What is known about a pack besides its contents (kept next to it as JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackMeta {
    pub format: u32,
    pub hx_edit_version: String,
    pub created: String,
    /// File name and byte size of each HX Edit file the pack was made from.
    pub sources: Vec<(String, u64)>,
}

pub fn read_pack_meta(meta_json: &str) -> Result<PackMeta, String> {
    serde_json::from_str(meta_json).map_err(|e| format!("pack meta: {e}"))
}

#[derive(Serialize)]
struct PackOut<'a> {
    format: u32,
    sym: &'a [SymEntry],
    models: BTreeMap<&'a String, &'a ModelInfo>,
    category: BTreeMap<&'a String, &'a (String, Rgb)>,
    groups: &'a [PickGroup],
}

#[derive(Deserialize)]
struct PackIn {
    sym: Vec<SymEntry>,
    models: HashMap<String, ModelInfo>,
    category: HashMap<String, (String, Rgb)>,
    groups: Vec<PickGroup>,
}

impl Catalog {
    /// The catalog as pack bytes (the same catalog gives the same bytes).
    pub fn to_pack(&self) -> Result<Vec<u8>, String> {
        let out = PackOut {
            format: PACK_FORMAT,
            sym: &self.sym,
            models: self.models.iter().collect(),
            category: self.category.iter().collect(),
            groups: &self.groups,
        };
        let json = serde_json::to_vec(&out).map_err(|e| format!("pack: {e}"))?;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&json).map_err(|e| format!("pack: {e}"))?;
        gz.finish().map_err(|e| format!("pack: {e}"))
    }

    pub fn from_pack(bytes: &[u8]) -> Result<Catalog, String> {
        let mut json = Vec::new();
        const LIMIT: u64 = 32 * 1024 * 1024;
        flate2::read::GzDecoder::new(bytes).take(LIMIT + 1).read_to_end(&mut json).map_err(|e| format!("effects list pack: {e}"))?;
        if json.len() as u64 > LIMIT {
            return Err("the effects list is too large".into());
        }
        let doc: serde_json::Value = serde_json::from_slice(&json).map_err(|e| format!("effects list pack: {e}"))?;
        let format = doc.get("format").and_then(serde_json::Value::as_u64).ok_or("effects list pack: no format number")?;
        if format > u64::from(PACK_FORMAT) {
            return Err(format!("this effects list was made by a newer ToneStrand (format {format}); update the app"));
        }
        let p: PackIn = serde_json::from_value(doc).map_err(|e| format!("effects list pack: {e}"))?;
        Ok(Catalog::assemble(CatalogSource::Builtin { hx_edit_version: String::new() }, p.sym, p.models, p.category, p.groups))
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::test_support::mini_hx_edit_dir;
    use crate::catalog::{format_value, Catalog};
    use std::io::Write;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn pack_round_trip_synthetic() {
        let dir = mini_hx_edit_dir();
        let a = Catalog::load(&dir).unwrap();
        assert_eq!(a.sym.len(), 3);
        assert!(!a.groups.is_empty());
        let b = Catalog::from_pack(&a.to_pack().unwrap()).unwrap();
        assert_eq!(a.sym, b.sym);
        assert_eq!(a.groups, b.groups);
        for n in 0..a.sym.len() as i64 {
            assert_eq!(a.model_name(n), b.model_name(n));
            assert_eq!(a.category(n), b.category(n));
            assert_eq!(a.paired_cab(n), b.paired_cab(n));
            assert_eq!(a.dsp_cost(n), b.dsp_cost(n));
        }
        assert_eq!(a.paired_cab(0), Some(1));
        assert!(matches!(b.source(), crate::catalog::CatalogSource::Builtin { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pack_round_trip_matches_hx_edit() {
        let Some(dir) = crate::catalog::find_dir(None) else { return };
        let a = Catalog::load(&dir).unwrap();
        let b = Catalog::from_pack(&a.to_pack().unwrap()).unwrap();
        assert_eq!(a.model_count(), b.model_count());
        // The stored data must match exactly (floats included), not just as formatted text.
        assert!(a.sym == b.sym, "sym differs");
        assert!(a.models == b.models, "models differ");
        assert!(a.category == b.category, "category differs");
        assert!(a.groups == b.groups, "groups differ");
        for n in 0..a.sym.len() as i64 {
            assert_eq!(a.model_name(n), b.model_name(n));
            assert_eq!(a.dsp_cost(n), b.dsp_cost(n));
            for i in 0..a.sym[n as usize].params.len() + 1 {
                let (na, pa) = a.param(n, i);
                let (nb, pb) = b.param(n, i);
                assert_eq!(na, nb);
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    for v in [pa.min, pa.default, pa.max] {
                        assert_eq!(format_value(Some(&pa), v), format_value(Some(&pb), v), "model {n} param {i}");
                    }
                }
            }
        }
        assert_eq!(a.picker_groups().len(), b.picker_groups().len());
    }

    #[test]
    fn from_pack_rejects_garbage() {
        assert!(Catalog::from_pack(b"not gzip").is_err());
        assert!(Catalog::from_pack(&gzip(b"{\"format\":1}")).is_err());
        let a = Catalog::load(&mini_hx_edit_dir()).unwrap().to_pack().unwrap();
        assert!(Catalog::from_pack(&a[..a.len() / 2]).is_err());
        assert!(Catalog::from_pack(&gzip(b"{not json")).is_err());
        assert!(Catalog::from_pack(&gzip(b"{\"sym\":[],\"models\":{},\"category\":{},\"groups\":[]}")).is_err());
    }

    #[test]
    fn from_pack_rejects_newer_format() {
        let err = Catalog::from_pack(&gzip(b"{\"format\":99,\"sym\":[],\"models\":{},\"category\":{},\"groups\":[]}")).err().unwrap();
        assert!(err.contains("newer"), "{err}");
    }

    #[test]
    fn pack_meta_reads() {
        let m =
            super::read_pack_meta(r#"{"format":1,"hx_edit_version":"3.8","created":"2026-11-19","sources":[["Helix.sym",12]]}"#).unwrap();
        assert_eq!(m.sources, vec![("Helix.sym".to_owned(), 12)]);
        assert!(super::read_pack_meta("{").is_err());
    }
}
