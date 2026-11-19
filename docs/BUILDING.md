# Building ToneStrand

## Toolchain

- **Rust stable**, edition 2021. Developed with rustc 1.100 on `x86_64-pc-windows-gnu`.
  Any recent stable toolchain should work; `rustup update stable` if a dependency complains.
- Windows: either toolchain works.
  - `x86_64-pc-windows-gnu` (what it is developed with) needs a MinGW-w64 GCC on `PATH`
    for linking (for example from MSYS2: `pacman -S mingw-w64-x86_64-gcc`).
  - `x86_64-pc-windows-msvc` needs the Visual Studio Build Tools ("Desktop development with C++").
- macOS: Xcode command-line tools (`xcode-select --install`).
- Linux: not tried yet. HX Edit is not needed to run ToneStrand: without it the effect list built into the program
  (`catalogs/`) is used. (The Windows and macOS builds need nothing more than the above.)

No other system libraries: USB uses the pure-Rust `nusb` crate (no libusb), the WebSocket to
the ToneStrand Box link uses `tungstenite`, and box discovery uses `mdns-sd`.

So far it has only been built and used on **Windows 11**; CI builds macOS but nobody has run it. The code has macOS/Linux branches
(file manager, link opening, window level) but those builds haven't been tried; expect small
fixes there.

## Build and run

```sh
cargo build --release          # target/release/tonestrand(.exe)
cargo run --release
```

Debug builds are optimised (`opt-level = 1`, dependencies at 2) so the UI stays smooth while
developing: `cargo run`.

Offline builds work once the crates are in the local cache: `cargo build --offline`.

### Patched crates (`vendor/`)

`Cargo.toml` patches two crates with local copies:

- `vendor/webbrowser`: a small stand-in that opens links with the system's default handler
  (Windows `ShellExecuteW`, macOS `open`, Linux `xdg-open`). The published crate links
  `shlwapi`, which the self-contained GNU toolchain doesn't ship. **When it can go:** on the
  MSVC toolchain (`x86_64-pc-windows-msvc`) `shlwapi` is available, so remove the `webbrowser`
  line under `[patch.crates-io]` in `Cargo.toml` and delete this folder; then build once with
  the MSVC toolchain and check that a link in the window still opens. Keep it for GNU builds.
- `vendor/autocfg`: a copy of the upstream crate, kept so offline builds resolve.

If you prefer the upstream crates, remove the `[patch.crates-io]` section and the `vendor/`
folder.

### Assets

`build.rs` packs every PNG in `assets/models/icons` into the program
(`OUT_DIR/photos.rs`), so the executable works on its own wherever it is copied. Logos, the
effect-type icon sheet and `models.json` are included with `include_bytes!`/`include_str!`
from `src/`. Adding a photo: drop `<name>.png` into that folder and reference it from
`assets/models/models.json`.

## Tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check   # run `cargo fmt` to fix
```

121 tests (two crates, `cargo test --workspace`): the session against a fake pedal (`fakepedal.rs`: handshake order, refusals,
Ignored on purpose: `cargo test -- --ignored recycles_to_the_bin` moves a scratch file into the real Windows Recycle Bin.
timeouts, chunked writes, half-finished transfers, pings), the MessagePack codec, the pedal document round trip, `.hlx` editing,
snapshots and controllers, unit conversion, IR/NAM conversion (byte-for-byte against the NAM
A2 Lite project's own converter, using `tests/data/`), icons, catalog value formatting.
Some tests read HX Edit's files when present and skip themselves otherwise; the built-in effects list tests always run.

## Checking the UI without a person

```sh
tonestrand --screenshot shot.png
```

Starts normally, draws about 120 frames, saves the window as a PNG and exits. The window
opens without taking focus. Combine with the environment variables below to picture a given
page or file.

| Variable | Effect |
|---|---|
| (none) | `--screenshot` always opens the sample preset and changes nothing in your settings file |
| `TONESTRAND_PAGE` | Start on `settings`, `irs`, `tones`, `tests`, `library` or `open` |
| `TONESTRAND_OPEN` | With `TONESTRAND_PAGE=open`: open this preset file at start |
| `TONESTRAND_UNIT` | Start for this unit's device number (e.g. `2162694` HX Stomp) |
| `TONESTRAND_LIBRARY` | Use this preset library folder |
| `TONESTRAND_TONES` | Use this IR/NAM library folder |
| `TONESTRAND_REAL_NAMES` | `1`: show real gear names |
| `TONESTRAND_NAM` | `1`: turn the NAM A2 Lite support on |
| `TONESTRAND_SIZE` | `1400x1100`: the window size (points) for a picture |
| `TONESTRAND_SELECT` | With `TONESTRAND_OPEN`: the place in the chain to select (0 is the input) |
| `TONESTRAND_CTL_TAB` | The Controllers card's tab to start on (0 Footswitches ... last = Command Center) |
| `TONESTRAND_TABS` | Press Tab this many times in a picture (to see where the keyboard focus goes) |

## Read-only command-line tools

Handy when working on formats or the protocol; none of them writes to the pedal.

| Command | What it does |
|---|---|
| `--probe [host]` | Connect (USB, or a paired ToneStrand Box at `host`), list setlists/presets, disconnect |
| `--find-boxes` | Look for boxes on the network (mDNS and the `.local` names) and print their status |
| `--list-models` | Print the effects list in use (HX Edit's, else the built-in one): category, variant, name |
| `--read-backup <file>` | List the presets inside an `.hxb` or `.hls` |
| `--dump-doc <file>` | Print a pedal document's structure (`.hxpreset`) |
| `--commands <file or slot> [host]` | Print a preset's Command Center as the pedal stores it (a `.hxpreset`, or a slot such as `42B` read from the pedal) |
| `--cc-compare <file or slot> <file.hlx> [host]` | Line an HX Edit `.hlx` export up with the pedal's Command Center: slot positions and which argument holds each field |
| `--cc-raw <host> <slot 0-12> <id> [args]` | WRITES one Command Center slot of the loaded preset into the pedal's edit buffer (not saved), for trying argument layouts |
| `--cc-reload <host>` | Reload the loaded preset from flash, dropping edit-buffer changes (wait a few seconds after another tool; the pedal needs a moment between sessions) |
| `--dump-loaded <host> <file>` | Save the loaded preset's edit buffer (unsaved changes included) to a `.hxpreset` file; reads only |
| `--put-slot <host> <slot> <file.hxpreset> <name>` | WRITES a saved pedal document back to a slot (undo a test) |
| `--cc-snap <host> <snapshot> <slot> [args]` | WRITES one command's values in one snapshot into the edit buffer (not saved) |
| `--send-hlx <host> <slot> <file.hlx> <backup folder>` | WRITES a `.hlx` to a slot as the app does, after saving the slot's document to the folder; reports whether it reads back the same |
| `--box-proxy <listen address:port> <box address[:port]> [folder]` | A logging pass-through: point another editor at this computer instead of the box, and see the pedal's requests and replies it makes (documents are saved to the folder) |
| `--ctl-check <files>` | Read and rewrite each document's controllers; must stay byte-identical |
| `--attach-check <files>` | Move split/merge positions in memory and read them back |
| `--dsp-report <files>` | Per-path DSP load of pedal documents |
| `--hlx-check <files>` | Convert `.hlx` files to pedal documents and report |
| `--to-hlx <file.hxpreset> <out.hlx> [device]` | A pedal preset as a `.hlx`, optionally made for another unit (reads and writes files only) |
| `--fmt-check` | Print value formatting for every catalog parameter |
| `--find-blocks <files>` | Find which documents use given models |
| `--nam2ir <out> <files>` | Convert NAM captures to User IR `.wav` files (writes files, not the pedal) |

Run `tonestrand --help` for the full list. In release builds on Windows the output appears in
the terminal the tool was started from.

## Where things are stored

- Settings: `<config dir>/ToneStrand/settings.json` (Windows: `%APPDATA%\ToneStrand`).
- Preset library: `Documents/ToneStrand Presets` by default (changeable in Settings), with
  `IRs and NAM` inside for the IR/NAM library and `Self-test backups` for the self-test.
