# Working on ToneStrand (for coding agents)

Read this first, then `docs/ARCHITECTURE.md`. The protocol and formats are in
`docs/PROTOCOL.md` and `docs/FILE_FORMATS.md`.

## Commands

```sh
cargo build                     # debug (optimised enough to use)
cargo build --release
cargo test                      # 58 tests; must stay green
cargo fmt                       # rustfmt.toml sets the style; CI runs `cargo fmt --check`
cargo clippy --all-targets -- -D warnings   # must stay clean (CI runs these)
cargo run -- --help             # the command-line tools
cargo run -- --screenshot shot.png   # render the window to a PNG and exit
```

On Windows, if linking fails with "Permission denied" on an `.o` file, the folder is being
synced (OneDrive) or the exe is still running: close `tonestrand.exe` and build again.

## Ground rules

1. **The pedal is the person's instrument. Never risk their presets or IRs.**
   - Every write to a slot is preceded by a backup of that slot (`device::backup_slot`), and
     every IR import or delete by a backup of the IR (`device::backup_ir`). If no backup can
     be made, nothing is written.
   - Documents go to the pedal through `DevicePreset::encode_checked`, never `encode`.
   - Files on disk are written with `files::write_atomic`; new files that must not replace
     an existing one get their name from `files::unique_path`.
   - Wait 750 ms after flash writes (`session::SETTLE_FLASH`); don't stack writes.
   - If a transfer fails half way, stop sending (the pedal may be mid-transfer); the
     self-test does this via `Run::lost`.
   - Through a Helrig Box, pace chunks more gently (`Wire::slow`).
   - Never send NAM captures to a pedal without the NAM A2 Lite firmware.
2. **Documents must round-trip byte for byte.** Don't re-encode values with a different
   MessagePack format; extend `msgpack.rs` rather than bypassing it. Run `--ctl-check` /
   the round-trip tests on real backups when touching `devpreset.rs`.
3. **Don't add Line 6 files.** Effect names, parameters and ranges come from the person's HX
   Edit installation at run time (`catalog.rs`). The only Line 6 material kept in the repo is
   the effect photos and descriptions (`assets/models/`) and the type icons
   (`assets/block Images/`), by the owner's decision, under the README's "Third-party material
   and trademarks" notice (removed on request via an issue). Don't add more.
4. **Keep the UI plain-spoken.** Labels and messages are short, everyday words. Tips go behind
   a `theme::help` "?" icon, not as visible small text.
5. **Font gaps:** egui's default fonts lack `●`, `→`, `▾`, `↶`, `↷` (they draw as squares). Paint
   shapes or use words.
6. **One look, two palettes.** Use `theme::bg()/panel()/card_bg()/card_hi()/line()/ink()/muted()/wash()`
   and `theme::accent()`/`on_accent()`, never fixed greys or teal: they follow the dark or light
   palette (Settings > Display). `theme.rs` has a test that text reads on every surface in both. (There used to be a second, "Classic" look; it is gone, so
   don't add a switch for it.)
7. **Settings stay compatible:** `Settings` has a struct-level `#[serde(default)]`; give a new
   field its value in `impl Default for Settings` (older files then load with it).
8. **Units differ.** Use `snapshots::family_of_device(unit)` for paths, lanes, slot limits
   (HX Stomp shares 8 slots between both lanes), snapshot counts, footswitches, expression
   pedals and FX loops (4 on Helix Floor/Rack, 2 on Helix LT, HX Effects and HX Stomp/XL).

## How to verify a change

- `cargo fmt`, `cargo test` and `cargo clippy --all-targets -- -D warnings` (CI runs all three on pull requests).
- A change to `session.rs`: add a case to `fakepedal.rs` (a fake pedal behind `Wire`).
- A screenshot (`--screenshot`) of the page you changed; use the `TONESTRAND_*` variables in
  `docs/BUILDING.md` to open a given page or file.
- Anything that talks to the pedal: run the hidden self-test (Ctrl+Shift+T) on a spare slot,
  over USB and, if relevant, through a Helrig Box. Say plainly what wasn't tried on hardware.

## Where to change what

| Want to change | Look at |
|---|---|
| Chain drawing, gap menu, keyboard | `chain.rs` |
| Block panel, knobs, pickers | `editors/mod.rs` (shared widgets, picker), `editors/device.rs` (pedal documents + controllers card), `editors/hlx_file.rs` (.hlx), `editors/studio.rs` (top-bar strips) |
| Window layout, tabs, dialogs, saving | `window/mod.rs` |
| Top bars, status bar, left panel | `window/bars.rs` |
| Connecting, pairing, device events | `window/connection.rs` |
| Library / IRs / Settings / Tests pages | `window/library_page.rs`, `window/irs.rs`, `window/settings.rs`, `window/tests_page.rs` |
| Restore from a slot backup (`Pedal backups`), window-wide shortcuts, drag and drop | `window/backups.rs`, `window/mod.rs` (`shortcuts`, `drop_files`) |
| Command-line tools | `cli.rs` |
| A pedal operation | `session.rs` (request), `device.rs` (Cmd/Event), `proto.rs` (opcode) |
| Pedal document fields | `devpreset.rs` (+ `docs/FILE_FORMATS.md`) |
| `.hlx` editing | `hlx.rs`, `snapshots.rs`, `controllers.rs`, `units.rs` |
| Effect names / formatting | `catalog.rs`, `realnames.rs` |
| Colours, widgets, icons | `theme.rs`, `icons.rs` |

## Known gaps and unverified areas

- Some live operations haven't been tried on a pedal: knob changes on a paired cab
  (`26: 1`) and controller min/max/CC live edits. Snapshot copy on the live preset uses the
  pedal's own ops 90/91 (checked on an HX Stomp only).
- Sends/returns: drawn as loop markers; placing them uses the nearest free slot. A return
  placed before its send is flagged, not prevented.
- macOS builds in CI but has never been run; Linux is not supported (no HX Edit files for the effect list).
- Command Center on the pedal document (checked on an HX Stomp only): a trigger grid (Instant 1-6, FS 1-5, EXP 1-2) with
  a per-trigger editor for every command type, a snapshot selector (each snapshot keeps its own values) and
  a Momentary switch for CC Toggle and Hotkey. Slot order is from live capture. On the loaded preset a
  change is written to the pedal's own requests where they suffice (ops 67/68/58, checked by reading back, else the edit buffer, op 21) and kept by "Save on pedal". A `.hlx` sent to the
  pedal carries its Command Center. The other units use a per-unit slot table from the documentation (not checked
  on a unit; see docs/ROADMAP.md). Not built: ops 67/68/88 live (op 21 is used).
  `docs/COMMAND_CENTER.md` has every type's field order and the live requests.
- Device-preset lower lanes are drawn packed; `.hlx` lower lanes are still drawn by stored
  position.
- The DSP estimate is calibrated so 126 real HX Stomp presets all read ≤ 97%; it is an
  estimate, not the pedal's own figure.
- `.hlx` → pedal conversion uses the loaded preset as the template; units other than the
  HX Stomp haven't been tried.

## Ideas queued by the owner

- Send/return representation: refine the loop drawing (jack numbers per unit, stereo loops).
- Polish of the look (spacing, colours) as the owner reviews screenshots.
