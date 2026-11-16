# Architecture

ToneStrand is one Rust binary built on eframe/egui (immediate-mode UI, OpenGL via glow).
The window runs on the main thread; all talk with the pedal happens on one background
thread. Everything else is plain data structures edited in place every frame.

```
            ┌───────────────────────── main thread (egui) ─────────────────────────┐
            │ window/    App: tabs of Doc { kind: Hlx(json) | Device(DevicePreset) }│
            │   ├─ editors/     chain + block panel + controllers (per Doc kind)    │
            │   ├─ chain.rs     signal-path diagram (cells, hits, line menu)        │
            │   ├─ controllers.rs / snapshots.rs / units.rs   (.hlx editing)        │
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
| `window/` | `App` (`mod.rs`: tabs, the Studio layout, dialogs, saving/loading, undo/redo) and one file per part: `bars.rs` (top bars, status bar, left panel), `connection.rs` (device thread events, pairing, pedal cards), `library_page.rs`, `irs.rs`, `settings.rs`, `tests_page.rs`. Pages (Editor, Library, IRs, Settings, hidden Tests), dialogs, the Studio layout, connection handling, saving/loading, undo/redo. |
| `editors/` | The editor for both document kinds (`mod.rs`: shared widgets and the effect picker; `device.rs`: pedal documents and their controllers card; `hlx_file.rs`: `.hlx`; `studio.rs`: Studio top-bar strips): chain per path, block panel (photo, knobs, cab, footswitch), value widgets (knobs, typed values, IR/NAM slot pickers), effect picker window, controllers card for pedal documents, Studio top-bar strips (snapshot chips, BPM, path meters). |
| `chain.rs` | The diagram: `Chain { main, lower, split, merge, selected, ... }` → `show()` returns `Vec<Hit>` (click, drop, toggle, gap menu actions...). Draws effect boxes, loop markers for sends/returns, IN/OUT terminals, split/merge dots, keyboard shortcuts. |
| `controllers.rs` | Controllers panel for `.hlx` (footswitches, expression, snapshots, MIDI, Command Center). |
| `snapshots.rs` | `.hlx` snapshots (select, add, copy/paste, delete) and controller assignments; unit `Family` table (snapshot counts, footswitches, paths, FX loops, slot limits). |
| `hlx.rs` | `.hlx` JSON helpers: blocks, paths, split/merge, moving/placing models, blank presets per unit. |
| `units.rs` | Converting a `.hlx` to another unit with a report of everything that doesn't fit. |
| `devpreset.rs` | `DevicePreset`: the pedal's own document (parse/encode byte-for-byte), slots, values, snapshots, controllers, footswitches, split/merge attach, `.hlx` → pedal conversion. |
| `catalog.rs` | Effect catalog read from HX Edit's installed files: model numbers ↔ symbols, names, categories, colours, parameters with display formatting, DSP loads, picker groups. |
| `device.rs` | The pedal thread: `Cmd` (UI → thread) and `Event` (thread → UI); heartbeat, recovery after failures, IR kind sorting. |
| `session.rs` | One conversation with the pedal over any `Wire`: handshake, services, chunked sends, acks, request/reply matching, high-level operations (presets, slots, live edits, IRs, controllers). |
| `proto.rs` | Frame and channel header encoding, opcode and key constants, slot labels. |
| `msgpack.rs` | MessagePack codec that remembers each value's original format byte so documents re-encode identically. |
| `wire.rs` | `Wire` trait; `UsbWire` (Line 6 vendor 0x0e41, bulk 0x01/0x81) and `BoxWire` (Helrig Box `ws://<box>/raw`). |
| `boxes.rs` | Helrig Box discovery (mDNS), pairing, relay mode, settings file (`Settings`). |
| `backup.rs` | Reading HX Edit `.hxb` backups and `.hls` setlists. |
| `irs.rs` | WAV ↔ pedal IR (2,048 float32 samples at 48 kHz); NAM capture detection. |
| `nam.rs` | NAM A2 Lite support: block knob mapping and the `.nam` → IR converter. |
| `files.rs` | Safe writes: `write_atomic` (temp file + rename) and `unique_path` / `copy_into` (never replace an existing file). |
| `library.rs`, `folders.rs`, `tonelib.rs` | Preset library and IR/NAM library as real folders on disk. |
| `fakepedal.rs` | Tests only: a fake pedal behind `Wire` for testing `Session` without hardware. |
| `selftest.rs` | Hidden Tests page: backs up, exercises every operation on one preset slot and one IR slot, restores, verifies; stops immediately if the link drops. |
| `theme.rs` | Palette (Studio vs Classic, switchable at run time), accent colour, cards, pills, help (?) icons, logos. |
| `icons.rs` | Effect-type icons cut from `assets/block Images/Blocks_stadium.png` (category line icons and per-kind tiles). `kind_label` names an amp carrying its cab "Amp+Cab" (a cab carrying a second cab "Dual Cab"); `paint_model` draws Amp+Cab as the amp tile with a small cab tile on its corner. |
| `modelinfo.rs` | Effect photos and descriptions (`assets/helixhelp`, from Line 6's release notes). For an amp + cab or dual cab, `side()` shows the cab's photo under the amp's. |
| `realnames.rs` | Optional real-gear names for effects. |

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
4. The device thread runs commands one by one; after structural changes it re-reads the live
   preset and sends `Event::Document`, which replaces the live tab's document.
5. The effect picker (`editors::picker_window`) is a separate window; its pick comes back to
   `App::apply_pick`, which places the model in the document (and sends `SetModel` live).

### Editor parts (Studio layout)

`editors::set_part(p)` makes an editor draw only part of itself:
`0` everything (Classic look), `1` chain only, `2` selected block's panel only,
`3` controllers only (window-side). The Studio layout (`App::studio_central`) draws the chain
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

- Two looks, switched in Settings: **Studio** (default; mockup-style top bar, flat effect
  boxes with icons, docked block panel, round IN/OUT terminals) and **Classic**.
  `chain::studio()` is the global switch; `theme::bg()/panel()/card_bg()/card_hi()/line()`
  return the palette for the current look; `theme::accent()` is the chosen accent colour.
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
