//! What the two document kinds (a `.hlx` JSON file and the pedal's own document) have in
//! common, so the window can treat an open preset the same way whichever it is.
//!
//! The trait holds only what really is the same job done twice: saving, tempo and the path
//! meters. Editing blocks stays per kind: the pedal keeps numbered slots with gaps and sends
//! live commands, `.hlx` keeps keyed blocks per path; see the notes in the pull request.

use super::*;

pub trait PresetDoc {
    /// File extension used when saving (no dot).
    fn file_ext(&self) -> &'static str;
    /// The bytes to save. The pedal's document re-encodes byte for byte; `.hlx` is pretty JSON.
    fn encode(&self) -> Vec<u8>;
    /// Tempo in BPM, when the document has one.
    fn tempo(&self) -> Option<f64>;
    fn set_tempo(&mut self, bpm: f64);
    /// Each path: (blocks used, blocks it can hold, estimated DSP %, None without a catalog).
    fn path_meters(&self, cat: Option<&Catalog>) -> Vec<(usize, usize, Option<f64>)>;
}

impl PresetDoc for serde_json::Value {
    fn file_ext(&self) -> &'static str {
        "hlx"
    }

    fn encode(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    fn tempo(&self) -> Option<f64> {
        hlx::tempo(self)
    }

    fn set_tempo(&mut self, bpm: f64) {
        hlx::set_tempo(self, bpm)
    }

    fn path_meters(&self, cat: Option<&Catalog>) -> Vec<(usize, usize, Option<f64>)> {
        let unit = snapshots::family(self);
        let dsp = cat.map(|c| hlx_dsp_use(c, self));
        hlx::dsps(self)
            .into_iter()
            .take(unit.paths)
            .map(|d| {
                let used = dsp.as_ref().and_then(|v| v.iter().find(|(k, _)| *k == d).map(|(_, u)| *u));
                (hlx::block_count(self, &d) as usize, unit.max_blocks as usize, used)
            })
            .collect()
    }
}

impl PresetDoc for DevicePreset {
    fn file_ext(&self) -> &'static str {
        "hxpreset"
    }

    fn encode(&self) -> Vec<u8> {
        DevicePreset::encode(self)
    }

    fn tempo(&self) -> Option<f64> {
        DevicePreset::tempo(self)
    }

    fn set_tempo(&mut self, bpm: f64) {
        DevicePreset::set_tempo(self, bpm)
    }

    fn path_meters(&self, cat: Option<&Catalog>) -> Vec<(usize, usize, Option<f64>)> {
        let fam = snapshots::family_of_device(crate::nam::unit());
        let slots = self.slots();
        (0..fam.paths as i64)
            .map(|p| {
                let in_path: Vec<&SlotView> = slots.iter().filter(|s| s.path == p).collect();
                let count = in_path.iter().filter(|s| s.kind == Kind::Block).count();
                (count, fam.max_blocks as usize, cat.map(|c| slots_dsp(c, &in_path)))
            })
            .collect()
    }
}

/// The estimated DSP use of some slots, in percent (a cab riding with an amp included).
pub(super) fn slots_dsp(cat: &Catalog, slots: &[&SlotView]) -> f64 {
    slots.iter().map(|s| s.model.map(|m| dsp_percent(cat, m)).unwrap_or(0.0) + s.paired.map(|m| dsp_percent(cat, m)).unwrap_or(0.0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hlx_goes_through_the_trait_unchanged() {
        let mut json = hlx::sample();
        let d: &mut dyn PresetDoc = &mut json;
        assert_eq!(d.file_ext(), "hlx");
        assert_eq!(d.encode(), serde_json::to_vec_pretty(&hlx::sample()).unwrap());
        let before = d.tempo();
        assert_eq!(before, hlx::tempo(&hlx::sample()));
        d.set_tempo(111.0);
        assert_eq!(hlx::tempo(&json), Some(111.0));
        let meters = json.path_meters(None);
        assert!(!meters.is_empty() && meters.iter().all(|m| m.2.is_none()));
    }

    #[test]
    fn device_goes_through_the_trait_byte_for_byte() {
        let doc = DevicePreset::parse(&DevicePreset::tiny_for_tests().encode()).unwrap();
        let d: &dyn PresetDoc = &doc;
        assert_eq!(d.file_ext(), "hxpreset");
        assert_eq!(d.encode(), doc.encode());
        assert_eq!(d.tempo(), doc.tempo());
        assert_eq!(d.path_meters(None).len(), snapshots::family_of_device(crate::nam::unit()).paths);
    }
}
