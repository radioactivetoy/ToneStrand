# Android phase 1: core/UI split and catalog export — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the pedal and document code a UI-free crate, and replace the hard need for HX Edit with an effect
catalog converted from HX Edit, stored in the repo per HX Edit version and built into the app — all on Windows,
with the desktop app unchanged.

**Architecture:** A Cargo workspace: a new `tonestrand-core` crate in `core/` holds protocol, documents, catalog,
device thread and files (no `eframe`/`egui` dependency, so the compiler enforces the line); the existing
`tonestrand` crate keeps the window, editors and CLI and re-exports core modules at its root so `crate::session`
style paths keep working. The catalog gains a compact pack format (gzip JSON) written by `--export-catalog` and
embedded by `build.rs` from the newest `catalogs/<version>/`.

**Tech Stack:** Rust stable, eframe/egui 0.31 (app crate only), serde/serde_json, flate2 (already a dependency).

**Spec:** `docs/superpowers/specs/2026-11-19-android-app-design.md` (sections 1, 4, 7; phase 1 in section 8).

## Global Constraints

- Branch `android`. Never commit to `main`.
- At every commit: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green on Windows.
- The Windows app looks and behaves as before: the screenshots from Task 1 Step 1 must match at the end (Task 6).
- AGENTS.md rules 1-8 apply; rule 3 is rewritten in Task 4 (owner's decision: converted catalogs are kept in the repo).
- Desktop backups stay in `<library>/Pedal backups` (not touched in this phase).
- No `nusb` upgrade in this phase (held until the USB phase).
- First catalog: HX Edit **3.82** from `C:\Program Files (x86)\Line6\HX Edit\res` (the owner's install).
- Library, folders, IR library (`library.rs`, `folders.rs`, `tonelib.rs`), `modelinfo.rs`, `icons.rs`, `theme.rs`, `realnames.rs` stay in the app crate (library storage is phase 6).
- Tests that need HX Edit skip themselves when `catalog::find_dir(None)` is `None` (as the existing ones do); CI has no HX Edit.

## Review Focus

1. **A corrupt or truncated embedded pack** (bad gzip, bad JSON, unknown `format`): the app must start with no
   catalog and the usual "no effects list" state, never panic. → test `from_pack_rejects_garbage` (Task 3).
2. **A pack written by a newer ToneStrand** (`format` > ours): refused with a plain message, not misread. →
   test `from_pack_rejects_newer_format` (Task 3).
3. **HX Edit installed but older than the built-in catalog** (fewer models in `Helix.sym`): the built-in one is used,
   so models the pedal has are named. → test `choose_prefers_newer_builtin` (Task 5).
4. **A custom HX Edit folder in Settings that no longer holds `Helix.sym`**: falls back to the built-in catalog
   instead of an empty editor. → test `choose_falls_back_when_folder_missing` (Task 5).
5. **Display formatting drift after the round trip** (a knob showing `380 ms` before and something else after): the
   pack must keep every field `format_value` uses. → test `pack_round_trip_matches_hx_edit` compares
   `format_value` output for every parameter at min, default and max (Task 3).

---

## File structure

| Path | Responsibility |
|---|---|
| `Cargo.toml` | Adds `[workspace] members = [".", "core"]`; app depends on `tonestrand-core = { path = "core" }`. |
| `core/Cargo.toml` | `tonestrand-core`: serde, serde_json, tungstenite, mdns-sd, nusb, ureq, dirs, flate2, png (only what core files use). **No eframe.** |
| `core/src/lib.rs` | `pub mod` for each core module. |
| `core/src/{backup,boxes,catalog,cmdslots,cmdtool,compare,device,devpreset,devpreset_to_hlx,files,hlx,irs,msgpack,nam,proto,routes,selftest,session,snapshots,support,units,wire}.rs`, `core/src/fakepedal.rs` (cfg(test)) | Moved from `src/` unchanged except for the edits named below. |
| `core/src/cmdslots.rs` | New: Command Center data (slot kinds, slot tables, command specs, key helpers) cut out of `src/controllers.rs`. |
| `core/src/catalog/pack.rs` | New: pack format (serialize, parse, meta). |
| `core/build.rs` | New: embeds the newest `catalogs/<version>/catalog.json.gz`. |
| `catalogs/3.82/catalog.json.gz`, `catalogs/3.82/meta.json` | New: first converted catalog. |
| `src/main.rs` | Drops moved `mod`s; `pub use tonestrand_core::{...}` for each so `crate::x` paths resolve. |
| `src/controllers.rs` | Keeps UI; `pub use tonestrand_core::cmdslots::*`. |
| `src/editors/mod.rs` | `DSP_BUDGET` becomes `pub use tonestrand_core::catalog::DSP_BUDGET`. |
| `src/cli.rs` | `--export-catalog`. |
| `src/window/mod.rs`, `src/window/settings.rs` | Load with `catalog::load_best`; Settings says which catalog is in use. |
| `.github/workflows/ci.yml` | `--workspace` flags. |
| `AGENTS.md`, `README.md`, `docs/ARCHITECTURE.md`, `docs/BUILDING.md`, `CHANGELOG.md` | Rule 3, third-party notice, crate layout, commands. |

---

### Task 1: Cut the core's references to UI code

**Files:**
- Modify: `src/catalog.rs` (lines 15, 106-123, 188-194, 636-637), `src/editors/mod.rs:245`, `src/units.rs:78`
- Modify: `src/controllers.rs` (Command Center data, ~lines 305-790 and 1023-1120), `src/devpreset.rs:1346,1360`, `src/devpreset_to_hlx.rs:361-365`, `src/support.rs:56`, `src/units.rs:98`, `src/cmdtool.rs:8`
- Create: `src/cmdslots.rs`
- Modify: every UI use of `PickGroup.colour` / `Catalog::category*` (13 sites; `grep -rn "\.colour\b\|category(\|category_of_symbol" src`)

**Interfaces:**
- Produces: `catalog::Rgb(pub u8, pub u8, pub u8)` with `#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]`; `PickGroup.colour: Rgb`; `Catalog::category(&self, i64) -> Option<(String, Rgb)>`; `Catalog::category_of_symbol(&self, &str) -> Option<(String, Rgb)>`; `catalog::DSP_BUDGET: f64 = 80.0`.
- Produces: `theme::rgb(c: catalog::Rgb) -> egui::Color32` (app side, the only conversion point).
- Produces: `cmdslots::{SlotKind, CmdSlot, command_name, slots_for_family, active_commands, device_slots, unit_slots, layout_verified, hlx_arg_keys, unit_slot_of_hlx_key}` with today's signatures; `controllers` re-exports them.

- [ ] **Step 1: Record the desktop baseline (before any change)**

On `android` at the design commit, build release and render these to `target/baseline/` (create it):
`TONESTRAND_SIZE=1320x820` for each of: editor with `tests/data` sample `.hlx` open and `TONESTRAND_SELECT=3`;
`TONESTRAND_PAGE=library`; `TONESTRAND_PAGE=irs`; `TONESTRAND_PAGE=settings`; the same editor shot with
`TONESTRAND_THEME=light`; the editor shot at `TONESTRAND_CTL_TAB=0`.
Command per shot: `./target/release/tonestrand.exe --screenshot target/baseline/<name>.png`.
Expected: 6 PNGs. Keep the exact commands in `target/baseline/commands.txt` for Task 6.

- [ ] **Step 2: Write the failing test** in `src/catalog.rs` tests

```rust
#[test]
fn colour_from_reads_hex() {
    assert_eq!(colour_from("0x8899aa"), Rgb(0x88, 0x99, 0xaa));
    assert_eq!(colour_from("nonsense"), colour_from("0x8899aa"));
}
```
(Read `colour_from` first; if its fallback for bad input differs, assert what it does today.)

- [ ] **Step 3: Run it to see it fail** — `cargo test colour_from_reads_hex` → compile error, `Rgb` not found.

- [ ] **Step 4: Implement**: add `Rgb`, make `colour_from` return it, change the category map and `PickGroup.colour` to `Rgb`, remove `use eframe::egui::Color32`. Add `pub const DSP_BUDGET: f64 = 80.0;` in `catalog.rs`; `editors/mod.rs` re-exports it; `units.rs` and `catalog.rs` use `crate::catalog::DSP_BUDGET`. Add `theme::rgb` and use it at the 13 UI sites.

- [ ] **Step 5: Move the Command Center data** from `controllers.rs` to `src/cmdslots.rs`: `SlotKind`, `CmdSlot`, `FieldSpec`, `CmdSpec` and their tables, and the pure functions listed in Interfaces (plus private helpers they need). No `egui`, `theme` or `Ui` in the new file. `controllers.rs` keeps `panel`, `command_center_ui`, `device_command_center` and other drawing code, with `pub use crate::cmdslots::*;`. Point `devpreset.rs`, `devpreset_to_hlx.rs`, `support.rs`, `units.rs`, `cmdtool.rs` at `crate::cmdslots`. Move the tests that cover the moved functions with them.

- [ ] **Step 6: Check no core file names UI modules**

Run: `grep -nE "egui|eframe|crate::(theme|editors|icons|window|chain|controllers|modelinfo|library|folders|tonelib)\b" src/{backup,boxes,catalog,cmdslots,cmdtool,compare,device,devpreset,devpreset_to_hlx,files,hlx,irs,msgpack,nam,proto,routes,selftest,session,snapshots,support,units,wire,fakepedal}.rs`
Expected: no output.

- [ ] **Step 7: Run everything** — `cargo fmt; cargo clippy --all-targets -- -D warnings; cargo test` → all pass (58+ tests).

- [ ] **Step 8: Commit** — `git commit -am "Core files no longer use UI code: catalog colours as Rgb, DSP budget in catalog, Command Center tables in cmdslots"` (add `src/cmdslots.rs`).

---

### Task 2: The `tonestrand-core` crate

**Files:**
- Modify: `Cargo.toml`, `src/main.rs`, `build.rs` (unchanged logic; check paths)
- Create: `core/Cargo.toml`, `core/src/lib.rs`
- Move (`git mv`): the 22 core files and `fakepedal.rs` from `src/` to `core/src/`

**Interfaces:**
- Consumes: Task 1's UI-free core files.
- Produces: crate `tonestrand_core` with `pub mod` for every moved module; the app's root has `pub use tonestrand_core::{backup, boxes, catalog, cmdslots, cmdtool, compare, device, devpreset, devpreset_to_hlx, files, hlx, irs, msgpack, nam, proto, routes, selftest, session, snapshots, support, units, wire};` so existing `crate::session::...` paths in app code compile unchanged.

- [ ] **Step 1: Move the files** with `git mv` (keeps history). Create `core/src/lib.rs` with the `pub mod` list and `#[cfg(test)] mod fakepedal;`.

- [ ] **Step 2: Create `core/Cargo.toml`** (`name = "tonestrand-core"`, `edition = "2021"`, `license = "GPL-3.0-or-later"`, `publish = false`) with only the dependencies the moved files use, copied with the same versions and features from the root `Cargo.toml`. Add `[workspace] members = [".", "core"]` and the path dependency to the root. Move `[patch.crates-io]` and profiles stay at the root (workspace-wide).

- [ ] **Step 3: Fix paths** that break: `include_str!`/`include_bytes!` inside moved files (prefix `../../`), `pub(crate)` items the app uses (make `pub`), items the app reaches that were private. Do not change behaviour.

- [ ] **Step 4: Prove the line holds**

Run: `cargo tree -p tonestrand-core -e normal | grep -i -E "eframe|egui|winit"`
Expected: no output.

- [ ] **Step 5: Run everything with `--workspace`** — `cargo fmt; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace` → same test count as Task 1 Step 7, all pass. `cargo run --release -- --help` prints the same help as before.

- [ ] **Step 6: Commit** — `git commit -m "Pedal and document code in its own crate (tonestrand-core) with no UI dependency"`.

---

### Task 3: Catalog pack format

**Files:**
- Create: `core/src/catalog/pack.rs` (turn `catalog.rs` into `catalog/mod.rs` + `pack.rs`, or keep `catalog.rs` with `#[path]`; builder's call)
- Modify: `core/src/catalog.rs` (derives, a shared constructor)
- Test: in `pack.rs`

**Interfaces:**
- Consumes: `Rgb` (Task 1).
- Produces:
  - `pub struct PackMeta { pub format: u32, pub hx_edit_version: String, pub created: String, pub sources: Vec<(String, u64)> }` (serde; `sources` = file name and byte size of each HX Edit file read).
  - `pub const PACK_FORMAT: u32 = 1;`
  - `impl Catalog { pub fn to_pack(&self) -> Result<Vec<u8>, String>; pub fn from_pack(bytes: &[u8]) -> Result<Catalog, String>; pub fn source(&self) -> &CatalogSource; }`
  - `pub enum CatalogSource { HxEdit(PathBuf), Builtin { hx_edit_version: String } }` replacing the `dir: PathBuf` field.
  - `pub fn read_pack_meta(meta_json: &str) -> Result<PackMeta, String>`.
- Pack bytes = gzip of JSON `{ "format": 1, "sym": [...], "models": {...}, "category": {...}, "groups": [...] }`. `sym_index` and `nam_info` are not stored; they are rebuilt by one private constructor `Catalog::assemble(source, sym, models, category, groups) -> Catalog` that both `load` and `from_pack` call.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn pack_round_trip_synthetic() {
    let dir = tempdir_with_mini_hx_edit(); // writes a 3-model Helix.sym, one .models file, HelixControls.json, HX_ModelCatalog.json
    let a = Catalog::load(&dir).unwrap();
    let b = Catalog::from_pack(&a.to_pack().unwrap()).unwrap();
    assert_eq!(a.sym, b.sym);
    assert_eq!(a.groups, b.groups);
    for n in 0..a.sym.len() as i64 {
        assert_eq!(a.model_name(n), b.model_name(n));
        assert_eq!(a.category(n), b.category(n));
        assert_eq!(a.paired_cab(n), b.paired_cab(n));
    }
}

#[test]
fn pack_round_trip_matches_hx_edit() {
    let Some(dir) = crate::catalog::find_dir(None) else { return };
    let a = Catalog::load(&dir).unwrap();
    let b = Catalog::from_pack(&a.to_pack().unwrap()).unwrap();
    assert_eq!(a.model_count(), b.model_count());
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
}

#[test]
fn from_pack_rejects_newer_format() {
    let err = Catalog::from_pack(&gzip(b"{\"format\":99,\"sym\":[],\"models\":{},\"category\":{},\"groups\":[]}")).unwrap_err();
    assert!(err.contains("newer"), "{err}");
}
```
The mini HX Edit fixture is written by the test into a temp folder from strings in the test (made-up symbols such as `TS_TestAmp`, never copied from HX Edit). `gzip` is a small test helper over `flate2`.

- [ ] **Step 2: Run to see them fail** — `cargo test -p tonestrand-core pack_` → compile errors (`to_pack` missing).

- [ ] **Step 3: Implement**: `#[derive(Serialize, Deserialize, PartialEq)]` on `DisplayPiece`, `DisplaySpec`, `Param`, `ModelInfo`, `SymEntry`, `PickModel`, `PickGroup`; `assemble`; `to_pack`/`from_pack` (error text for a newer format: `"this effects list was made by a newer ToneStrand (format {n}); update the app"`); `CatalogSource` in place of `dir`.

- [ ] **Step 4: Run tests** — `cargo test --workspace` → all pass (`pack_round_trip_matches_hx_edit` actually runs on the owner's machine; say so in the commit if it skipped).

- [ ] **Step 5: Commit** — `git commit -m "Effects list pack format: the catalog saved as gzip JSON and read back identically"`.

---

### Task 4: `--export-catalog`, the first catalog, and rule 3

**Files:**
- Modify: `src/cli.rs` (command and `--help` line)
- Create: `catalogs/3.82/catalog.json.gz`, `catalogs/3.82/meta.json`, `catalogs/README.md`
- Modify: `AGENTS.md` (rule 3), `README.md` ("Requires HX Edit" box, Requirements, "Third-party material and trademarks"), `CHANGELOG.md`

**Interfaces:**
- Consumes: `Catalog::load`, `Catalog::to_pack`, `PackMeta` (Task 3).
- Produces: `tonestrand --export-catalog <hx edit res folder> <catalogs folder> <hx edit version>` → writes `<catalogs folder>/<version>/catalog.json.gz` and `meta.json` with `files::write_atomic`; refuses (message, exit code 1) if `<version>` folder already exists.

- [ ] **Step 1: Write the failing test** in `src/cli.rs` tests (or a function the CLI calls, e.g. `cli::export_catalog(res: &Path, out: &Path, version: &str) -> Result<PathBuf, String>`)

```rust
#[test]
fn export_catalog_writes_pack_and_meta_and_refuses_overwrite() {
    let res = tonestrand_core::catalog::test_support::mini_hx_edit_dir(); // the Task 3 fixture, made pub under cfg(any(test, feature = "test-support")) or duplicated
    let out = tempdir();
    let dir = export_catalog(&res, &out, "9.99").unwrap();
    let meta = read_pack_meta(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta.hx_edit_version, "9.99");
    assert_eq!(meta.format, PACK_FORMAT);
    assert!(Catalog::from_pack(&std::fs::read(dir.join("catalog.json.gz")).unwrap()).is_ok());
    assert!(export_catalog(&res, &out, "9.99").is_err());
}
```

- [ ] **Step 2: Run to see it fail** — `cargo test export_catalog` → not found.

- [ ] **Step 3: Implement `export_catalog`** and the `--export-catalog` argument; `created` is today's date (use `support.rs`'s date stamp helper).

- [ ] **Step 4: Run tests** — `cargo test --workspace` → pass.

- [ ] **Step 5: Make the first catalog**

Run: `cargo run --release -- --export-catalog "C:\Program Files (x86)\Line6\HX Edit\res" catalogs 3.82`
Expected: `catalogs/3.82/catalog.json.gz` (a few hundred KB) and `meta.json`. Check: `du -h catalogs/3.82/*`.

- [ ] **Step 6: Rewrite the rules and notice** (plain words, AGENTS rule 4 style):
  - `AGENTS.md` rule 3: Line 6 material in the repo is the effect photos/descriptions, the type icons, **and the effects lists converted from HX Edit in `catalogs/`** (by the owner's decision, same notice). New lists are made only with `--export-catalog` from an HX Edit install; nothing else from HX Edit is added.
  - `README.md`: HX Edit is no longer required (it is used when installed and newer); Requirements updated; "Line 6 material" gains a bullet for `catalogs/` (names, knob names and ranges, categories converted from HX Edit's resource files), same removal-on-request notice.
  - `catalogs/README.md`: what the folder is, how to add a version (the command above), that the app builds in the newest.
  - `CHANGELOG.md`: an "Unreleased" entry.

- [ ] **Step 7: Commit** — add `catalogs/`, `src/cli.rs`, docs: `git commit -m "--export-catalog and the first built-in effects list (HX Edit 3.82); rule 3 and the README say so"`.

---

### Task 5: Built-in catalog when HX Edit is missing or older

**Files:**
- Create: `core/build.rs`
- Modify: `core/src/catalog.rs` (`builtin`, `choose`, `load_best`), `src/window/mod.rs:338,1005`, `src/cli.rs:65-69,834`, `src/window/settings.rs:95-115`

**Interfaces:**
- Consumes: `from_pack`, `CatalogSource` (Task 3), `catalogs/<version>/` (Task 4).
- Produces:
  - `pub fn builtin() -> Option<(&'static str, &'static [u8])>` — (HX Edit version, pack bytes) of the newest folder in `catalogs/` by version order (split on `.`, compare numerically), embedded by `core/build.rs` (`cargo:rerun-if-changed=../catalogs`); `None` when the folder is empty.
  - `pub enum Pick { Local, Builtin, None }` and `pub fn choose(local_symbols: Option<usize>, builtin_symbols: Option<usize>) -> Pick` — Local when only local exists or `local >= builtin`; Builtin when only builtin exists or `builtin > local`; None when neither. (`*_symbols` = `sym.len()`; `Helix.sym` only grows, so more symbols = newer. Designer's call: this replaces reading HX Edit's version, which the files don't record.)
  - `pub fn load_best(custom_hx_edit: Option<&str>) -> Option<Catalog>` — loads local (via `find_dir`) and built-in, applies `choose`, logs which was used.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn choose_prefers_newer_builtin() {
    assert_eq!(choose(Some(900), Some(950)), Pick::Builtin);
    assert_eq!(choose(Some(950), Some(950)), Pick::Local);
    assert_eq!(choose(Some(990), Some(950)), Pick::Local);
}

#[test]
fn choose_falls_back_when_folder_missing() {
    assert_eq!(choose(None, Some(950)), Pick::Builtin);
    assert_eq!(choose(Some(900), None), Pick::Local);
    assert_eq!(choose(None, None), Pick::None);
}

#[test]
fn builtin_catalog_loads() {
    let (version, bytes) = builtin().expect("catalogs/ has at least one version");
    assert!(!version.is_empty());
    let cat = Catalog::from_pack(bytes).unwrap();
    assert!(cat.model_count() > 100);
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p tonestrand-core choose_ builtin_` → not found.

- [ ] **Step 3: Implement** `core/build.rs`, `builtin`, `choose`, `load_best`. A built-in pack that fails `from_pack` is logged and treated as absent (never a panic).

- [ ] **Step 4: Use it**: replace the two `find_dir(...).and_then(Catalog::load)` calls in `window/mod.rs` and the ones in `cli.rs` with `catalog::load_best(...)`. The CLI's "needs HX Edit's files" message becomes "no effects list found" (only reachable when `catalogs/` was empty at build time).

- [ ] **Step 5: Settings text**: under "Effects list", one plain line saying which list is in use — `Built into ToneStrand (HX Edit 3.82)` or `From HX Edit (<folder>)` — from `Catalog::source()`. The help text: "Effect and amp names, knob ranges and colours come from HX Edit's files when it is installed and up to date, otherwise from the list built into ToneStrand." Keep the folder picker.

- [ ] **Step 6: Run everything** — `cargo fmt; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace` → pass.

- [ ] **Step 7: Check the fallback by hand**: set `hx_edit` in Settings to an empty folder, restart, open the sample preset: blocks show names and knobs; Settings says "Built into ToneStrand (HX Edit 3.82)". Put the setting back ("Use the usual place"). Screenshot the Settings page to `target/check/settings-builtin.png`.

- [ ] **Step 8: Commit** — `git commit -m "The built-in effects list is used when HX Edit is missing or older; Settings says which is in use"`.

---

### Task 6: CI, docs, and the Windows check

**Files:**
- Modify: `.github/workflows/ci.yml`, `.github/workflows/release.yml` (only if it calls cargo with paths that changed), `docs/ARCHITECTURE.md`, `docs/BUILDING.md`, `AGENTS.md` (Commands, "Where to change what")

- [ ] **Step 1: CI**: `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace --all-targets` in both jobs. Note in the comment that the built-in catalog tests now run in CI (no HX Edit needed).

- [ ] **Step 2: Docs**: ARCHITECTURE gets the two-crate picture (core has no UI dependency; app re-exports), the `cmdslots` module, the catalog sources and `catalogs/`; BUILDING and AGENTS commands gain `--workspace`; AGENTS "Where to change what" points pedal/document rows at `core/src/`. Test count in AGENTS updated to the real number.

- [ ] **Step 3: Windows check against the baseline**

Run `cargo build --release`, then each command in `target/baseline/commands.txt` writing to `target/after/`.
Then: `for f in target/baseline/*.png; do cmp -s "$f" "target/after/$(basename $f)" && echo "same $f" || echo "DIFF $f"; done`
Expected: `same` for all six. For any `DIFF`, open both images and confirm the difference is only the Settings "Effects list" line from Task 5; anything else is a regression to fix before committing.

- [ ] **Step 4: Run the app normally** (`cargo run --release`), connect to the Box or USB if available, open a preset, turn a knob. Say plainly in the hand-off which of these were tried; pedal checks are the owner's (the device code moved but did not change).

- [ ] **Step 5: Final gate** — `cargo fmt --check; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace` → all pass.

- [ ] **Step 6: Commit** — `git commit -am "CI and docs for the two-crate layout and the built-in effects list"`.
