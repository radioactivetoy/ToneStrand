//! A made-up mini "HX Edit" folder for tests (here and in the app crate). Every symbol is invented; nothing is copied from HX Edit.

use std::path::PathBuf;

/// Write a small HX Edit-shaped resource folder (a `Helix.sym`, one `.models` file, `HelixControls.json` and
/// `HX_ModelCatalog.json`) into a fresh folder under the system temp folder and return its path.
pub fn mini_hx_edit_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("tonestrand-mini-hx-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    w(
        "Helix.sym",
        r#"[
  {"symbol": "TS_TestAmp", "parameters": ["TS_Drive", "TS_Mode"]},
  {"symbol": "TS_TestCab", "parameters": ["TS_Size"]},
  {"symbol": "TS_TestDelayStereo", "parameters": ["TS_Time", "TS_Mix"]}
]"#,
    );
    w(
        "Test.models",
        r#"{"models": [
  {"symbolicID": "TS_TestAmp", "name": "Test Amp", "load": 12.5, "cablink": "TS_TestCab",
   "params": [
     {"symbolicID": "TS_Drive", "name": "Drive", "valueType": 1, "displayType": "ts_percent", "min": 0, "max": 1, "default": 0.5},
     {"symbolicID": "TS_Mode", "name": "Mode", "valueType": 0, "displayType": "ts_mode", "min": 0, "max": 2, "default": 1}
   ]},
  {"symbolicID": "TS_TestCab", "name": "Test Cab", "load": 7,
   "params": [{"symbolicID": "TS_Size", "name": "Size", "valueType": 1, "displayType": "ts_percent", "min": 0, "max": 1, "default": 0.25}]},
  {"symbolicID": "TS_TestDelay", "name": "Test Delay", "mono": false, "stereo": true, "load": 20, "load_stereo": 30,
   "devices": [{"id": 2162944}],
   "params": [
     {"symbolicID": "TS_Time", "name": "Time", "valueType": 1, "displayType": "ts_time", "min": 0, "max": 2, "default": 0.5, "max_stereo": 4},
     {"symbolicID": "TS_Mix", "name": "Mix", "valueType": 1, "displayType": "ts_percent", "min": 0, "max": 1, "default": 0.3}
   ]}
]}"#,
    );
    w(
        "HelixControls.json",
        r#"{
  "ts_percent": {"dspToDisplayScale": 100, "formatUnits": "%.0f %%"},
  "ts_time": {"format": [{"lowerBound": 0, "upperBound": 1, "unitsMultiplier": 1000, "formatUnits": "%.0f ms"},
                          {"lowerBound": 1, "upperBound": 100, "unitsMultiplier": 1, "formatUnits": "%.2f s"}]},
  "ts_mode": {"format": ["Clean", "Crunch", "Lead"]}
}"#,
    );
    w(
        "HX_ModelCatalog.json",
        r#"{"categories": [
  {"name": "None", "color": "0x000000", "models": []},
  {"name": "Amp", "color": "0xaa5522", "models": [{"id": "TS_TestAmp", "name": "Test Amp"}]},
  {"name": "Cab", "color": "0x2255aa", "subcategories": [{"name": "Single", "models": [{"id": "TS_TestCab"}]}]},
  {"name": "Delay", "color": "0x22aa55", "subcategories": [{"name": "Stereo", "models": [{"id": "TS_TestDelay"}]}]}
]}"#,
    );
    dir
}
