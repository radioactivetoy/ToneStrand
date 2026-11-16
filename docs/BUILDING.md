# Building ToneStrand

## Toolchain

- **Rust stable**, edition 2021. Developed with rustc 1.100 on `x86_64-pc-windows-gnu`.
  Any recent stable toolchain should work; `rustup update stable` if a dependency complains.
- Windows: either toolchain works.
  - `x86_64-pc-windows-gnu` (what it is developed with) needs a MinGW-w64 GCC on `PATH`
    for linking (for example from MSYS2: `pacman -S mingw-w64-x86_64-gcc`).
  - `x86_64-pc-windows-msvc` needs the Visual Studio Build Tools ("Desktop development with C++").
- macOS: Xcode command-line tools (`xcode-select --install`).
- Linux: a C toolchain plus the development packages eframe/rfd need, for example on
  Debian/Ubuntu:
  `sudo apt install build-essential pkg-config libgtk-3-dev libxkbcommon-dev libgl1-mesa-dev libudev-dev`
  (GTK is for the file dialogs, OpenGL for drawing, udev for USB device discovery).
  USB access to the pedal needs a udev rule for vendor `0e41` (Line 6), e.g.
  `SUBSYSTEM=="usb", ATTR{idVendor}=="0e41", MODE="0666"` in `/etc/udev/rules.d/50-line6.rules`.

No other system libraries: USB uses the pure-Rust `nusb` crate (no libusb), the WebSocket to
the Helrig Box uses `tungstenite`, and box discovery uses `mdns-sd`.

So far it has only been built and used on **Windows 11**. The code has macOS/Linux branches
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
  `shlwapi`, which the self-contained GNU toolchain doesn't ship.
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
cargo test
cargo clippy --all-targets -- -D warnings
```

45 tests: the session against a fake pedal (`fakepedal.rs`: handshake order, refusals,
timeouts, chunked writes, half-finished transfers, pings), the MessagePack codec, the pedal document round trip, `.hlx` editing,
snapshots and controllers, unit conversion, IR/NAM conversion (byte-for-byte against the NAM
A2 Lite project's own converter, using `tests/data/`), icons, catalog value formatting.
Some tests read HX Edit's files when present and skip themselves otherwise.

## Checking the UI without a person

```sh
tonestrand --screenshot shot.png
```

Starts normally, draws about 120 frames, saves the window as a PNG and exits. The window
opens without taking focus. Combine with the environment variables below to picture a given
page or file.

| Variable | Effect |
|---|---|
| `TONESTRAND_PAGE` | Start on `settings`, `irs`, `tones`, `tests`, `library` or `open` |
| `TONESTRAND_OPEN` | With `TONESTRAND_PAGE=open`: open this preset file at start |
| `TONESTRAND_UNIT` | Start for this unit's device number (e.g. `2162694` HX Stomp) |
| `TONESTRAND_LIBRARY` | Use this preset library folder |
| `TONESTRAND_TONES` | Use this IR/NAM library folder |
| `TONESTRAND_REAL_NAMES` | `1`: show real gear names |
| `TONESTRAND_NAM` | `1`: turn the NAM A2 Lite support on |

## Read-only command-line tools

Handy when working on formats or the protocol; none of them writes to the pedal.

| Command | What it does |
|---|---|
| `--probe [host]` | Connect (USB, or a paired Helrig Box at `host`), list setlists/presets, disconnect |
| `--find-boxes` | Look for Helrig Boxes on the network and print their status |
| `--list-models` | Print the catalog read from HX Edit (category, variant, name) |
| `--read-backup <file>` | List the presets inside an `.hxb` or `.hls` |
| `--dump-doc <file>` | Print a pedal document's structure (`.hxpreset`) |
| `--ctl-check <files>` | Read and rewrite each document's controllers; must stay byte-identical |
| `--attach-check <files>` | Move split/merge positions in memory and read them back |
| `--dsp-report <files>` | Per-path DSP load of pedal documents |
| `--hlx-check <files>` | Convert `.hlx` files to pedal documents and report |
| `--fmt-check` | Print value formatting for every catalog parameter |
| `--find-blocks <files>` | Find which documents use given models |
| `--nam2ir <out> <files>` | Convert NAM captures to User IR `.wav` files (writes files, not the pedal) |

Run `tonestrand --help` for the full list. In release builds on Windows the output appears in
the terminal the tool was started from.

## Where things are stored

- Settings: `<config dir>/ToneStrand/settings.json` (Windows: `%APPDATA%\ToneStrand`).
- Preset library: `Documents/ToneStrand Presets` by default (changeable in Settings), with
  `IRs and NAM` inside for the IR/NAM library and `Self-test backups` for the self-test.
