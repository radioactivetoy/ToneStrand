# ToneStrand

A desktop preset editor for Line 6 Helix-family pedals (HX Stomp, HX Stomp XL, HX Effects,
Helix Floor / LT / Rack), connected over **USB** or wirelessly through a **Helrig Box**.

Written in Rust with [egui](https://github.com/emilk/egui) / eframe. One self-contained
program; no account, no cloud, nothing leaves your computer.

## What it does

- **Live editing** of the preset loaded on the pedal: every knob, switch, effect swap, move,
  split/merge and snapshot change is heard at once; "Save on pedal" keeps it.
- **Offline editing** of `.hlx` files (the JSON format Helix editors export) and of the pedal's
  own documents (`.hxpreset`), with undo/redo, and sending them to any slot (the slot is backed
  up first).
- **Signal chain** drawn as a diagram: drag to move, click the line between effects to split
  there or add an effect / FX loop / send / return, right-click menus, keyboard shortcuts
  (Space on/off, Enter change, Delete remove, arrows to select).
- **Snapshots**: recall, rename, copy/paste, duplicate, delete; per-snapshot knob values.
- **Controllers**: footswitches, expression pedals, snapshots, MIDI CC and (for `.hlx`) Command
  Center, both on files and live on the pedal.
- **Preset library** on disk (real folders), import of HX Edit backups (`.hxb`) and setlists
  (`.hls`), conversion between units with a report of what doesn't fit.
- **User IRs**: list, upload (any `.wav`, resampled to the pedal's format), download, rename,
  remove; IRs and NAM captures are told apart.
- **NAM A2 Lite** (optional community firmware for HX Stomp / XL / HX Effects): the block's
  knobs and capture picker, and a converter from `.nam` captures to the `.wav` the block loads.
- Effect photos and descriptions, real-gear names (optional), effect-type icons, DSP estimate
  per path, a "Studio" look (default) and the earlier "Classic" look.
- A hidden **self-test** page (Ctrl+Shift+T) that exercises every pedal operation on one slot
  and one IR slot, with backups and automatic restore.

## Requirements

- **HX Edit installed** on the same computer. ToneStrand reads effect names, knob names and
  ranges from HX Edit's resource files at start-up (`Helix.sym`, `*.models`,
  `HelixControls.json`, ...). Nothing from HX Edit is copied into this repository. If HX Edit
  is in an unusual place, set its folder in Settings.
- A Helix-family pedal on USB, or a paired Helrig Box with the pedal plugged into it.
- To build: Rust (stable). See [docs/BUILDING.md](docs/BUILDING.md).

## Quick start

```sh
cargo run --release
```

The release binary is `target/release/tonestrand(.exe)`.

## Documentation

| File | For |
|---|---|
| [docs/BUILDING.md](docs/BUILDING.md) | Toolchains, platform requirements, build and test commands |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the program is organised: modules, threads, data flow, UI |
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | The pedal's USB protocol and the Helrig Box relay |
| [docs/FILE_FORMATS.md](docs/FILE_FORMATS.md) | Preset documents, `.hlx`, `.hxb`/`.hls`, IR `.wav`, NAM conversion |
| [AGENTS.md](AGENTS.md) | Working rules and checklists for coding agents (Claude, Antigravity, ...) |

## Status

Used daily on an HX Stomp, over USB and through a Helrig Box (live editing, IR upload and
preset writes have been exercised with the self-test). Things not yet tried on real hardware
are listed in [AGENTS.md](AGENTS.md#known-gaps-and-unverified-areas).

## Third-party material

- **Effect photos and descriptions** (`assets/helixhelp/`) come from Line 6's own firmware
  release notes, as collected by helixhelp.com. Like the icons below, they are Line 6 material.
- **Effect-type icons** (`assets/block Images/`) are Line 6 artwork (from HX Edit / Helix
  Stadium). They belong to Line 6; make sure you may redistribute them before publishing the
  repository publicly, or replace them with your own.
- **Real gear names** are trademarks of their owners and are used only to describe the sound.
- The NAM converter follows the published file layout of
  [hx-stomp-nam-lite](https://github.com/nocaseguy/hx-stomp-nam-lite) (GPL-3; layout read, no
  code copied). The Helrig Box relay follows the box's public relay protocol description (MIT).
- Line 6, Helix, HX Stomp and HX Edit are trademarks of Yamaha Guitar Group; this project is not
  affiliated with or endorsed by Line 6 or by the makers of the Helrig Box.

## License

ToneStrand is free software: you can redistribute it and/or modify it under the terms of the
GNU General Public License as published by the Free Software Foundation, either version 3 of
the License, or (at your option) any later version. See [LICENSE](LICENSE).

The licence covers this project's own code and documentation. It does not cover the Line 6
material listed under "Third-party material" (effect icons, photos and descriptions), which
belongs to Line 6.
