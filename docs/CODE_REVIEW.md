# Code review — ToneStrand 0.2.0 (2026-11-16)

Scope: the whole crate (~20k lines of Rust in `src/`), docs, build setup and assets.
State at review time: `cargo test` passes (28/28). `cargo clippy` could not run
(component not installed on the `stable-x86_64-pc-windows-gnu` toolchain). Nothing here was
tried on a pedal; findings come from reading the code.

Severity: **High** = can lose the owner's data or break a project rule;
**Medium** = a real bug or fragile code; **Low** = cleanup / style.

---

## 1. Safety and data-loss issues (High)

> **Status:** H1–H8 fixed on branch `fix/data-safety` (see its commit). H4 (IR backups) and
> the pedal-write paths haven't been tried on hardware yet. The window-close prompt was
> checked by screenshot only, not by clicking the window's close button.

| # | Where | Problem | Suggested fix |
|---|---|---|---|
| H1 | `src/device.rs:491` | `backup_slot` returns `Ok(None)` and **lets the write go ahead** when `BACKUP_DIR` is unset. Ground rule 1 says every slot write is preceded by a backup; here the guarantee depends on `scan_library()` having run first. | Fail closed: return `Err("no backup folder set; nothing was written")` when the dir is `None`. Better, pass the backup dir in `Cmd::WriteSlot`/`WriteHlx` instead of a global. |
| H2 | `src/devpreset.rs:124` | `encode()` falls back to the **old offset table** when `computed_table()` returns `None` (a `TABLE_ORDER` key missing). That document would then go to the pedal with a stale table. `write_slot` re-encodes through this path. | Make `encode()` return `Result`, or have `write_slot` refuse when `computed_table()` is `None`. |
| H3 | `src/devpreset.rs:106` | `parse()` reads magic, table and tone, then **throws away any extra values** after them. Round-tripping such a blob silently changes it (breaks ground rule 2). | Reject a blob with trailing values, or keep them and write them back. |
| H4 | `src/device.rs:415`, `:460` | `ImportIrs` writes over occupied IR slots and `DeleteIrs` removes IRs **without a backup**, unlike presets. Ground rule 1 covers IRs too. | Before each import/delete, `ir_audio(slot)` → write `<backup>/IRs/<slot> <name> - before overwrite <stamp>.wav`. |
| H5 | `src/window.rs:1557` | Closing a tab with unsaved changes (`dirty`) **drops it without asking**. Closing the window does the same: there is no `close_requested` / `CancelClose` handling anywhere. | Ask "Save changes to X?" (Save / Don't save / Cancel) on tab close, and on window close via `ctx.input(|i| i.viewport().close_requested())` + `ViewportCommand::CancelClose`. |
| H6 | `src/window.rs:2680`, `src/tonelib.rs:217` | Library and IR-library **Import overwrite existing files with the same name** (`fs::copy` onto `dir.join(name)`). `save_to_library` already avoids clashes with `" (2)"`; imports don't. | Reuse the "(n)" naming from `save_to_library` (`window.rs:2830`) for imports, or ask. |
| H7 | `src/window.rs:536`, `:723`, `:2837`, `src/boxes.rs:141`, `src/nam.rs:442` | All saves use `fs::write` directly over the target. A crash or power cut mid-write leaves a **truncated preset or settings file**. | Small helper `write_atomic(path, bytes)`: write `path.tmp`, `sync_all`, `rename` over the target. Use it everywhere. |
| H8 | `src/boxes.rs:129` | `Settings::load()` turns **any** parse error into defaults, then the next `save()` overwrites the file. One bad field and the user's box pairings, folders and options are gone with no message. | On a parse error, rename the file to `settings.json.bad` and tell the user. Return `Result` from `save()` and show failures. |

## 2. Bugs and fragile code (Medium)

> **Status:** M1–M15 fixed on branch `fix/medium` (built on `fix/data-safety`). M12 replaces
> the per-frame encoding with a value comparison; the per-frame clone of the document stays.
> M8 keeps writing when the pedal *refuses* BEGIN (as before, now with a note) but stops
> when the link doesn't answer. Not tried on hardware: M1–M3, M8, M9, M11.

| # | Where | Problem |
|---|---|---|
| M1 | `src/wire.rs:145`, `:156` | `format!("{host}:80")` and `ws://{host}/raw` break for IPv6 addresses (needs `[addr]:80`). `boxes::start_discovery` falls back to an IPv6 address when a box has no IPv4 one (`boxes.rs:178`). Parse with `IpAddr` and add brackets, or build a `SocketAddr` directly. |
| M2 | `src/wire.rs:114` | After a failed USB read, the IN buffer is **not resubmitted**. The queue is then empty and the next `next_complete()` waits forever (or panics, depending on nusb's version). Today `Session::broken` stops most calls, but `drain()` and `idle()` still read. Resubmit before returning the error. |
| M3 | `src/wire.rs:102` | On a write timeout, `cancel_all()` leaves the cancelled transfer in the queue. The next `send()` picks up that transfer's "cancelled" status as its own result. Drain the cancelled completion before returning. |
| M4 | `src/main.rs:1` | `windows_subsystem = "windows"` in release builds means **every CLI tool** (`--probe`, `--nam2ir`, `--read-backup`, ...) prints nothing on Windows. Call `AttachConsole(ATTACH_PARENT_PROCESS)` when CLI flags are present, or ship a separate `tonestrand-cli` binary. |
| M5 | `src/main.rs:58`, `:162`, `:179`, `:207` | CLI tools `expect()`/panic on a missing or bad file instead of printing an error. |
| M6 | `src/msgpack.rs:452` | Decoding recurses without a depth limit. A corrupt or hostile `.hxpreset` with deeply nested arrays (`0x91 0x91 ...`) **overflows the stack and crashes the app**. Add a depth counter (64 is plenty). |
| M7 | `src/session.rs:861` | `wire_name` silently drops non-ASCII characters ("Café Lead" → "Caf Lead") and cuts at 16. The UI should show this before writing (see U5). |
| M8 | `src/session.rs:521`, `:812` | The result of `op::BEGIN` before slot/IR writes is ignored (`let _ =`). If BEGIN is refused, the write goes ahead anyway. At least log it; consider stopping. |
| M9 | `src/session.rs:458` | `select_preset` waits until `current == index` but never checks the **setlist**, so selecting index 5 in another setlist "succeeds" if index 5 is already loaded in the current one. |
| M10 | `src/device.rs:333` | `Cmd::Backup` stops at the first failed slot with `?`. The files already written stay, and the message doesn't say how far it got. Keep going, count failures, report "saved N, failed M". |
| M11 | `src/device.rs:246` vs `:372` | `WriteHlx` reads the slot back and compares; plain `WriteSlot` does not. Verify both the same way. |
| M12 | `src/window.rs:2525` | `track_history` runs every frame for the active document and, for pedal documents, calls `encode()` twice. Each `encode()` re-encodes the whole tone several times (`computed_table` encodes every value again). Plus the caller clones the whole `DocKind` every frame. Fine on small presets but wasteful; compare a cheap version counter or the `Value` trees directly (they derive `PartialEq`). |
| M13 | `src/window.rs:2559`, `:2571` | `import_backup` counts files it failed to write but never says why, and the summary uses `LogLevel::Good` even when 0 presets were imported (`window.rs:2688`). |
| M14 | `src/catalog.rs:174` | HX Edit lookup is hard-coded to `C:\Program Files...`. Use `%ProgramFiles%` / `%ProgramFiles(x86)%` (other drive letters, localised Windows). The macOS path is guessed (and listed as untried). |
| M15 | `src/device.rs:200` | Notification ids `20` and `1` (preset changed) are magic numbers; put them in `proto::` next to the opcodes. |

## 3. Bad practices and redundancy (Low)

> **Status:** done on branch `cleanup/section-3` (built on `fix/medium`), except the
> `PresetDoc` trait: `editors::DeviceEdit` is a first step, but unifying the two editors is a
> design change of its own. Also done: all 41 compiler warnings removed (mostly dead code),
> and `rustfmt` left out of CI because the code has never been formatted with it.

- **String errors with a sentinel.** `device.rs:218` uses `Err("__bye")` to mean "disconnect", then string-compares at `:473`. Use an enum (`enum Stop { Bye, Error(String) }`). Most of the crate uses `Result<_, String>`; a small `thiserror`-style enum per layer would keep messages but let code match on cause (`SessionError` already shows the pattern).
- **Global mutable state.** `device::BACKUP_DIR` (`device.rs:487`) is a `static Mutex`, set as a side effect of `App::scan_library` (`window.rs:341`). Pass the folder with the command.
- **Dead code.** `device.rs:471` `let _ = setlist;`: `setlist` is written in two places and never read. `window.rs:1541` `let dot = if d.is_live() { "" } else { "" };` is always `""`.
- **Duplicate helpers.** "Open a folder in the file manager" exists twice (`folders.rs:52` and `window.rs:3387`). `Settings::default()` (`boxes.rs:98`) repeats every `#[serde(default = ...)]` value by hand, so they can drift. Put `#[serde(default)]` on the struct and keep a single `Default` impl.
- **Parallel code per document kind.** `editors.rs` has `hlx_editor`/`device_editor`, `studio_strip_hlx`/`studio_strip_device`, `hlx_path_meters`/`device_path_meters`, `hlx_dsp_use`/`device_dsp`; `window.rs` matches on `DocKind` in many places. A small `trait PresetDoc` (blocks, paths, set_param, snapshots, encode) would remove much of that and keep the two kinds in step.
- **God files.** `window.rs` (3,492 lines, 63 functions) and `editors.rs` (3,102) hold the top bar, sidebar, every page and every dialog. Suggested split: `pages/{library,irs,settings,tests}.rs`, `dialogs.rs`, `connection.rs`.
- **Debug tools in `main.rs`.** ~270 lines of ad-hoc CLI parsing (`args.iter().position(...)`) for ten tools, with no `--help`. Move them to `cli.rs`. A `match` on the first argument is enough; no need for `clap`.
- **`#[allow(clippy::too_many_arguments)]`** in six places (`chain.rs:198`, `editors.rs:850`, `:1733`, `hlx.rs:175`, ...). These functions are asking for a small options struct.
- **Busy polling.** `UsbWire::send/recv` spin with `sleep(1ms)` and the device thread wakes every 40 ms even when idle. Not a bug, but it keeps a core warm on laptops. `block_on` with a timeout future would sleep properly.
- **Fixed sleeps on exit/pairing.** `on_exit` sleeps 300 ms (`window.rs:3447`) and hopes the device thread finishes. Join the thread with a timeout instead.
- **No CI and no lints.** Add `rustup component add clippy` and a GitHub Actions workflow (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`) on Windows. Note that several tests need HX Edit installed and should be `#[ignore]` (or skip cleanly) in CI.
- **Tests.** The protocol layer (`session.rs`, `device.rs`) has no tests. A `FakeWire` that replays recorded transfers (the `HX_DEBUG` hex logs are already in the right shape) would cover chunking, acks, ping handling and timeouts without a pedal.

## 4. Repository and licensing

> **Status:** the owner chose to keep the Line 6 images and descriptions. Branch
> `docs/third-party-notice` adds a README notice (free, non-commercial; © Line 6; removed on
> request via a GitHub issue) and a trademark disclaimer for Line 6 and real gear names. It also
> drops the helixhelp.com reference (the folder is now `assets/models/`). The notice doesn't
> amount to permission from Line 6; only a written yes from them would.

- `assets/models/` (423 photos plus `models.json`) and `assets/block Images/` are **Line 6 material**, and `tests/data/` holds a third-party NAM capture and IR. README says so, and AGENTS.md rule 3 says "Don't ship Line 6 files". `build.rs` also **embeds every photo in the binary**, so a released `.exe` ships them too.
  The repository was therefore pushed as **private**. Before going public: replace or remove those assets (or get written permission), and make `build.rs`/`modelinfo.rs` work without them.
- `assets/logos/` holds ~5 MB of source JPGs that the app doesn't need at run time. Keep them out of the repo or use Git LFS.
- `vendor/webbrowser` exists only to avoid `shlwapi` on the GNU toolchain. Worth a line in BUILDING.md on when it can go away (e.g., when building with MSVC).
- Box pairing tokens are stored in plain text in `settings.json`. That's acceptable for a local box, but the Windows Credential Manager (`keyring` crate) would be better.

## 5. Missing features and usability

> Later, on `feature/studio-only`: F2 rename and the footswitch-label counter were added, the Classic look was removed, the accent colour picker moved to Settings, and the code was formatted with rustfmt (checked in CI). Still open: U9 (compare), restore-all from a full backup, the per-frame document clone behind undo, plain-text Box tokens.

**Status (branch `feature/usability`):** done: U2, U3, U4, U5, U6, U7 (backup only; no restore-all), U8 (F2 rename added on `feature/studio-only`), U10, U11, U12, U13, U14. Not done: U9 (compare). U1 was done earlier. U3 uses the Windows shell directly (`files::delete_file`) instead of the `trash` crate; other systems delete for good after always asking. Needing a real pedal to check: U2 write-back, U7, U10.

| # | Idea | Why |
|---|---|---|
| U1 | **Unsaved-changes prompt** on tab and window close (see H5). | The most likely way to lose work today. |
| U2 | **Restore from backup.** Slots are backed up to `Pedal backups/`, but there's no "Restore this backup to its slot" action. Listing those files with the slot and date and offering "Write back" closes the loop. | Backups are only useful if restoring is easy. |
| U3 | **Delete to the Recycle Bin** (`trash` crate) instead of `remove_file` in the library and IR library. | "This cannot be undone" goes away. |
| U4 | **Drag and drop** `.hlx`/`.hxpreset`/`.hxb`/`.wav`/`.nam` files onto the window (`ctx.input(|i| i.raw.dropped_files)`) to open or import them. | Expected of a desktop editor; nothing handles dropped files today. |
| U5 | **Name field limits shown while typing**: a 16-char counter for presets (10 for snapshots, 26 for IRs), with unsupported characters flagged, instead of silently trimming them at send time. | Matches what lands on the pedal. |
| U6 | **First-run screen when HX Edit isn't found**: explain why it's needed, with a "Choose HX Edit folder..." button and a download link. | Without the catalog, most of the editor is empty. |
| U7 | **Full pedal backup** in one click: all setlists **and** all IRs to one folder, plus a matching "restore all" with a dry-run list. Today `Cmd::Backup` covers one setlist and no IRs. | One-step safety net before firmware updates. |
| U8 | **More shortcuts**: Ctrl+W close tab, Ctrl+O open, Ctrl+N new preset, Ctrl+Tab switch tabs, F2 rename. Undo/redo and Ctrl+S already exist. | Common editor muscle memory. |
| U9 | **Preset compare / diff** between two documents (or a document and its slot): blocks and knob values that differ. | Useful before "Write back to its slot". |
| U10 | **Reconnect automatically** when the same USB pedal reappears after `Disconnected`, instead of asking the user to connect again. | Unplugging and replugging is routine. |
| U11 | **Recent files** list and **reopen last session's tabs** (paths kept in `Settings`, with `#[serde(default)]`). | Saves clicks every start. |
| U12 | **Tab close target**: the "x" is a small text label. Make it a real button with a hover state and a tooltip. | Easier to hit, and clearer. |
| U13 | **Library search across setlists** and sorting by date modified. | Libraries grow quickly after importing `.hxb` backups. |
| U14 | **Log export**: a "Copy log" / "Save log" button for the status log (and the `HX_DEBUG` trace), for bug reports. | Most of the hardware behaviour can only be checked by users. |

## 6. What's good

- Clear layering (wire → session → device thread → UI). The pedal never blocks the window.
- The format-preserving MessagePack codec and the offset-table rebuild are a careful answer to
  a hard constraint, and they are tested.
- The safety habits are mostly there: slot backups, flash settle time, refusing to keep
  sending after a half-finished transfer (`Session::broken`), and the self-test with
  automatic restore.
- The docs (`ARCHITECTURE.md`, `PROTOCOL.md`, `FILE_FORMATS.md`, `AGENTS.md`) are unusually
  complete and match the code.

## Suggested order

1. H1, H2, H5, H7, H8: small changes that remove the data-loss paths.
2. H4, H6, M1–M3, M6.
3. Enable clippy and add CI; add a `FakeWire` test for the session.
4. U2, U3, U5, U6, then the refactors in section 3.

> **UX polish (branch `feature/ux-polish`):** effect names step down in size before being cut;
> one Backups menu; a block is selected automatically; the sample preset opens on the first start
> only; Unit shows as text when connected; Open + Recent are one menu; slightly lighter muted
> text and empty slots; Settings "Jump to" row; the window remembers its size and position;
> F1 shortcuts window; the Activity button turns red when something went wrong; a Getting
> started card; a Revert button and a brief "sent" label on the live preset. Not done (bigger
> features): compare presets, light theme, accessibility work, Box token storage.
