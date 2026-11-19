# Architecture

ToneStrand is a Rust workspace of two crates. `tonestrand-core` (`core/`) holds everything that does not
draw: the pedal link, documents, `.hlx` editing, the effects list. It has no UI dependency (the compiler
enforces it, so the same code can serve an Android app). The app crate (root, `src/`) is the egui window and
re-exports the core modules at its root, so code there says `crate::session`, `crate::devpreset` and so on.
The app is one binary built on eframe/egui (immediate-mode UI, OpenGL via glow).
The window runs on the main thread; all talk with the pedal happens on one background
thread. Everything else is plain data structures edited in place every frame.

```
            ┌───────────────────────── main thread (egui) ─────────────────────────┐
            │ window/    App: tabs of Doc { kind: Hlx(json) | Device(DevicePreset) }│
            │   ├─ editors/     chain + block panel + controllers (per Doc kind)    │
            │   ├─ chain.rs     signal-path diagram (cells, hits, line menu)        │
            │   ├─ controllers.rs   (.hlx and Command Center drawing)             │
            │   │             (.hlx editing itself: core/ hlx, snapshots, units)  │
            │   ├─ library.rs / folders.rs / tonelib.rs        (files on disk)      │
            │   └─ theme.rs / icons.rs / modelinfo.rs / realnames.rs (look, data)   │
            └───────────────▲──────────────────────────────┬──────────────────────┘
                     Event  │  (mpsc)                 Cmd  │
            ┌───────────────┴──────────────────────────────▼──────────────────────┐
            │ device.rs   background thread: owns the Session, runs one Cmd at a   │
            │             time, idles/keeps the link alive, reports Events          │
            │   session.rs  channels, requests/replies, acks, notifications         │
            │   proto.rs    frames, channel headers, opcodes, keys                  │
            │   msgpack.rs  format-preserving MessagePack                           │
            │   wire.rs     UsbWire (nusb bulk endpoints) | BoxWire (WebSocket)     │
            └─────────────────────────────────────────────────────────────────────┘
```

## Modules

| File | Role |
|---|---|
| `main.rs` | Entry point: window options, `--screenshot`; hands command-line tools to `cli.rs`. |
| `cli.rs` | The command-line tools (`--help` lists them; see BUILDING.md). |
| `window/` | `App` (`mod.rs`: tabs, the layout, dialogs, saving/loading, undo/redo) and one file per part: `bars.rs` (top bars, status bar with the Activity menu and problem reports, left panel, the first-connection notice), `connection.rs` (device thread events, pairing, pedal cards), `refresh.rs` (`RefreshGuard`: of several edits sent in a row only the newest copy of the live preset from the pedal is shown), `library_page.rs`, `irs.rs`, `settings.rs`, `tests_page.rs`. Pages (Editor, Library, IRs, Settings, hidden Tests), dialogs, the Studio layout, connection handling, saving/loading, undo/redo. |
| `editors/` | The editor for both document kinds (`mod.rs`: shared widgets and the effect picker; `device.rs`: pedal documents and their controllers card; `hlx_file.rs`: `.hlx`; `studio.rs`: Studio top-bar strips): chain per path, block panel (every block panel body is `BODY_H` tall: a photo tile, a divider and the knobs; an amp with a cab is two such sections with a line between; inputs and outputs have a jack picture and their own header tile), value widgets (knobs, typed values, IR/NAM slot pickers), effect picker window, controllers card for pedal documents, Studio top-bar strips (snapshot chips, BPM, path meters). |
| `chain.rs` | The diagram: `Chain { main, lower, split, merge, selected, ... }` → `show()` returns `Vec<Hit>` (click, drop, toggle, gap menu actions...). Draws effect boxes, loop markers for sends/returns, IN/OUT terminals, split/merge dots, keyboard shortcuts. |
| `controllers.rs` | Controllers panel for `.hlx` (footswitches, expression, snapshots, MIDI, Command Center), and the Command Center grid and editor for pedal documents (`device_slots` per unit, `device_command_center`, `hlx_arg_keys`). |
| `core/src/snapshots.rs` | `.hlx` snapshots (select, add, copy/paste, delete) and controller assignments; unit `Family` table (snapshot counts, footswitches, paths, FX loops, slot limits). |
| `core/src/hlx.rs` | `.hlx` JSON helpers: blocks, paths, split/merge, moving/placing models, blank presets per unit. |
| `core/src/units.rs` | Converting a `.hlx` to another unit with a report of everything that doesn't fit. |
| `core/src/devpreset_to_hlx.rs` | `DevicePreset::to_hlx`: a pedal document as a `.hlx` (the reverse of `from_hlx`), with a round-trip test over real presets; `guess_device` finds the unit a document came from. |
| `core/src/devpreset.rs` | `DevicePreset`: the pedal's own document (parse/encode byte-for-byte), slots, values, snapshots, controllers, footswitches, split/merge attach, Command Center (commands, Momentary, per-snapshot values, footswitch records), `.hlx` → pedal conversion (including Command Center). |
| `core/src/catalog/` | The effects list: model numbers ↔ symbols, names, categories, colours, parameters with display formatting, DSP loads, picker groups. `mod.rs` reads it from HX Edit's installed files or from the built-in pack; `load_best` picks HX Edit's when found and not older (by symbol count), else the built-in one (`CatalogSource` says which). `pack.rs` is the pack format (gzip JSON). `catalogs/<HX Edit version>/` holds the committed packs, made with `--export-catalog <res> <catalogs> <version>`; `core/build.rs` embeds the newest (by `catalog/version.rs`, shared with the build script). Known limit: "newer" is judged only by the number of models, because HX Edit's files carry no version; an HX Edit update that only changes knobs or ranges, with no new model, is not seen as newer (HX Edit wins a tie, so an installed copy of that update is still used). |
| `core/src/cmdslots.rs` | Command Center data without drawing: per-unit slot tables and field order (`controllers.rs` keeps the drawing and re-exports it). |
| `core/src/device.rs` | The pedal thread: `Cmd` (UI → thread) and `Event` (thread → UI); heartbeat, recovery after failures, IR kind sorting; `slot_op` copies, moves and swaps preset slots (backup first, read-back check, tested in `fakepedal.rs`); `change_command_center` (the pedal's own Command Center requests, read back and checked, falling back to writing the edit buffer). |
| `core/src/session.rs` | One conversation with the pedal over any `Wire`: handshake, services, chunked sends, acks, request/reply matching, high-level operations (presets, slots, live edits, IRs, controllers). |
| `core/src/proto.rs` | Frame and channel header encoding, opcode and key constants, slot labels. |
| `core/src/msgpack.rs` | MessagePack codec that remembers each value's original format byte so documents re-encode identically. |
| `core/src/wire.rs` | `Wire` trait; `UsbWire` (Line 6 vendor 0x0e41, bulk 0x01/0x81) and `BoxWire` (ToneStrand Box `ws://<box>/raw`). |
| `core/src/boxes.rs` | ToneStrand Box discovery (mDNS plus a repeated `.local` lookup that follows a box to a new address), pairing, relay status (incl. direct mode), relay mode, settings file (`Settings`). |
| `core/src/backup.rs` | Reading HX Edit `.hxb` backups and `.hls` setlists. |
| `core/src/irs.rs` | WAV ↔ pedal IR (2,048 float32 samples at 48 kHz); NAM capture detection. |
| `core/src/nam.rs` | NAM A2 Lite support: block knob mapping and the `.nam` → IR converter. |
| `core/src/files.rs` | Safe writes: `write_atomic` (temp file + rename) and `unique_path` / `copy_into` (never replace an existing file). |
| `library.rs`, `folders.rs`, `tonelib.rs` | Preset library and IR/NAM library as real folders on disk; the library also keeps tags (`.tonestrand-tags.json`, by file name) and searches names, effects, snapshot names and tags. |
| `core/src/support.rs` | What has been tried on which unit (only the HX Stomp so far): shown the first time a unit connects and put in problem reports; the report folder's date stamp. |
| `core/src/cmdtool.rs` | The Command Center command-line tools' logic (`--commands`, `--cc-compare`). |
| `boxproxy.rs` | `--box-proxy`: a logging pass-through to a ToneStrand Box, to see what another editor sends the pedal. |
| `core/src/fakepedal.rs` | Tests only: a fake pedal behind `Wire` for testing `Session` without hardware. |
| `core/src/selftest.rs` | Hidden Tests page: backs up, exercises every operation on one preset slot and one IR slot, restores, verifies; stops immediately if the link drops. |
| `theme.rs` | Palette, accent colour (picked in Settings), cards, pills, help (?) icons, logos. |
| `icons.rs` | Effect-type icons cut from `assets/block Images/Blocks_stadium.png` (category line icons and per-kind tiles; the dark edge of each tile is un-mixed so it does not show as a band on a coloured block). `kind_label` names an amp carrying its cab "Amp+Cab" (a cab carrying a second cab "Dual Cab"); `paint_model` draws Amp+Cab as the amp tile with a small cab tile on its corner. |
| `modelinfo.rs` | Effect photos and descriptions (`assets/models`, Line 6 material from its firmware release notes; see README). For an amp + cab or dual cab, `photo_tile()` draws one photo at a fixed size (the amp's, and the cab's in its own section); `about()` gives the description for the help pop-up. |
| `core/src/realnames.rs` | Optional real-gear names for effects. |

## Data model

- `App.docs: Vec<Doc>` is the list of open tabs; `App.active` the shown one.
- `Doc.kind`:
  - `DocKind::Hlx { json }`: a `.hlx` file kept as `serde_json::Value` (order preserved) and
    edited in place by `hlx.rs`, `snapshots.rs`, `units.rs`.
  - `DocKind::Device { preset: DevicePreset, origin }`: the pedal's own document.
    `origin` is `Live { setlist, index }` (the loaded preset: edits are also sent to the pedal as
    they happen), `Slot { .. }` (read from a slot, written back on request) or `File`.
- Undo/redo: `track_history(i, before, history)` compares the document before and after each
  frame's edits and pushes snapshots (merged when edits come within 700 ms). Live documents
  have no undo (their changes are already on the pedal).
- `Doc.selected`: the selected block. For pedal documents it is the index in
  `DevicePreset::slots()`; for `.hlx` it is the index in the editor's block list, or codes
  `10_000+path` (input), `20_000+path` (output), `40_000+path` (split), `50_000+path` (merge).

## Editing flow

1. `window/mod.rs` draws a tab and calls `editors::hlx_editor` or `editors::device_editor`.
2. The editor builds `chain::Chain` per path and calls `show()`; the returned `Hit`s are
   applied to the document (moves, toggles, picker requests, split/merge changes).
3. On a live pedal document each change also pushes a `Cmd` (`SetParam`, `SetEnabled`,
   `MoveBlock`, `MoveSplit`/`MoveJoin`, `AssignController`, ...). The window sends them to the
   device thread after the frame.
4. The device thread runs commands one by one (Command Center edits use the pedal's own requests and are checked by reading back); after structural changes it re-reads the live
   preset and sends `Event::Document`, which replaces the live tab's document. When several such edits are sent in a row, `window/refresh.rs` shows only the newest copy.
5. The effect picker (`editors::picker_window`) is a separate window; its pick comes back to
   `App::apply_pick`, which places the model in the document (and sends `SetModel` live).

### Editor parts

`editors::set_part(p)` makes an editor draw only part of itself:
`0` everything (the reset state), `1` chain only, `2` selected block's panel only,
`3` controllers only (window-side). The layout (`App::studio_central`) draws the chain
in a top panel sized to its measured height and the block/controllers panel below it, filling
the rest.

### The chain

- Main lane cells are positions; the lower lane exists when the path is really split
  (`hlx::split_active`, `DevicePreset::split_active`). Pedal documents draw the lower lane
  packed (effects in order, then one free place), because the pedal keeps them in numbered
  slots with gaps.
- Clicking the line between two places opens a menu: split here / move split / move merge /
  add an effect / add an FX loop, send or return. Sends, returns and FX loops are drawn as a
  loop marker above the line instead of an effect box (they still take a slot on the pedal).
- Block width follows the available width (`Chain.width`).

## The device thread

`device::connect(target, repaint)` spawns the thread and returns `Link { tx: Sender<Cmd>,
rx: Receiver<Event> }`. The loop:

- Waits up to 40 ms for a `Cmd`; when idle, calls `Session::idle` (reads, acks, collects
  notifications, keeps the Box link alive) and follows preset changes made on the pedal.
- Every 15 s of inactivity asks for the preset info (heartbeat). If that fails it re-opens the
  loaded preset; if that fails too it reports `Disconnected`.
- After any failed command it re-opens the loaded preset so the next live edit has a fresh link.
- Writes (slots, IRs) wait 750 ms after the reply so flash commits never stack.

## Look and feel

- One look: a mockup-style top bar, flat effect boxes with icons, a docked block panel and
  round IN/OUT terminals. (Function names with `studio` in them date from when there was also
  a "Classic" look, which has been removed.) `theme::bg()/panel()/card_bg()/card_hi()/line()` are the
  palette, with a dark and a light set (`theme::set_light`, chosen in Settings; `TONESTRAND_THEME=light` for
  one run); `theme::accent()` is the accent colour chosen in Settings.
- egui's default fonts lack some symbols (`●`, `→`, `▾`, `↶`); they render as squares. Paint
  dots/arrows with the painter, or use plain text.
- Tips go behind `theme::help(ui, text)` (a "?" icon), not as visible small text.
- Block types: an amp with its built-in cab shows as **Amp+Cab** (label and icon), a cab with
  a second cab as **Dual Cab**; use `icons::kind_label(category, has_paired_model)` wherever a
  block's type is shown, and `icons::paint_model` / `icons::show_model` to draw its icon.

## Settings and persistence

`boxes::Settings` is serialised to `<config dir>/ToneStrand/settings.json`: box pairings and
tokens, library folders, HX Edit folder, unit, look, accent, real names, NAM options, UI scale,
confirmations. New fields need `#[serde(default)]` so older files still load.

## Testing

- `cargo test`: unit tests per module (codec, document round trip, `.hlx` operations,
  snapshots, conversion, NAM converter against reference files, icons).
- Hardware: the hidden Tests page (Ctrl+Shift+T) runs the self-test against a real pedal.
- UI: `--screenshot` (see BUILDING.md).
