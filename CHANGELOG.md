# Changelog

## Unreleased

### New
- **Command Center layouts for the other units** (HX Stomp XL, HX Effects, Helix Floor / LT / Rack): the grid, the
  editor and the `.hlx` conversion follow a per-unit slot table, arranged like the unit's panel. These layouts come
  from the documentation and are marked "not checked on a pedal" in the editor and in the conversion notes until
  someone runs them on a unit (see docs/ROADMAP.md for how).
- docs/ROADMAP.md: what is left and in what order.
- **Library tags and wider search:** give a preset tags from its right-click menu (kept in a small file in the
  library folder, by file name, so they follow a preset between folders); click a tag to show those presets;
  search also looks in snapshot names and tags (`#tag` is exact).
- **First connection to a unit** shows what has been tried on it (only the HX Stomp so far) and how to report a problem.
- **Problem report** (Activity menu): a folder in Downloads with the log, version, unit and what is checked on it,
  and, if asked, the open preset. Nothing is sent anywhere.
- **The `.hlx` editor's amp and cab panel** is two same-size sections with a photo each, like the pedal editor.
- **Command Center changes on the loaded preset use the pedal's own requests** (op 67/48/33 for a type, 68 for a
  value, 58 for Momentary) instead of rewriting the whole preset, then check what the pedal holds and fall back to
  the whole write if it differs or the edit is for a snapshot the pedal has not selected (which would recall it).
  Lab tool: `--cc-live`.
- `--help` separates the read-only tools from the lab tools that write to the pedal; CI also builds on macOS and
  Linux (informational).

## 1.0.0-beta.3 - 2026-11-18

### New
- **Command Center on the pedal (HX Stomp)**, now complete: a Command Center tab in the Controllers card with a
  trigger grid (Instant 1-6, FS 1-5, EXP 1-2) and an editor per trigger for every command type (MIDI CC,
  CC Toggle, Note On, Bank/Program, MMC, Hotkey, HX Preset, HX Snapshot, HX Looper, pedal MIDI CC). Each snapshot
  keeps its own values (the preset's snapshot bar picks which), CC Toggle and Hotkey have a Momentary switch, and
  the footswitch gets its name from the command. On the loaded preset a change reaches the pedal at once (kept by
  "Save on pedal"). The slot order and every argument layout were checked against another editor's traffic on a pedal.
- **A `.hlx` sent to the pedal carries its Command Center** (waits held to the pedal's limit of 20, Momentary
  flags, each snapshot's own values).
- **Input and output panels** follow the layout of an effect's: icon tile, jack picture, one row of settings.
  Guitar In-Z and Guitar Pad moved from the Controllers tab to the Input block's panel.
- **Block panels have one height**; an amp with a cab is two sections of it, each with its own photo and a
  line between. A model's description is behind a (?) next to its name.
- Tools: `--commands`, `--cc-compare`, `--cc-raw`, `--cc-reload`, `--box-proxy` (log what another editor sends
  the pedal), `--dump-loaded`, `--send-hlx`, `--cc-snap`, `--put-slot` (see docs/BUILDING.md).

### Fixed
- A pedal document's Command Center was read from the first element of each row; the pedal's own rows start with 0
  and keep the type in a separate list, so every command except HX Preset showed as empty. Both are now read and
  written as the pedal does.
- Command Center footswitch labels: the pedal's FS 1 to FS 5 are slots 6 to 10 (FS 4 / FS 5 were wrong).
- Changing a Command Center command on the loaded preset could jump back to "None" or change again: the window
  was refreshed from the pedal after every edit and an older refresh overwrote the next change. It now keeps
  what you set and only reloads if the pedal kept something different. The same race affected the other live
  edits (guitar input, effects, moves, snapshots, controllers, footswitches): when edits are sent in a row, only the
  newest copy of the preset from the pedal is shown.

### Known gaps
- Command Center is read and edited on the HX Stomp only; other units get a note that it wasn't carried over.
- The names and defaults of the Bank/Program and HX Looper switches, and HX Looper's press and release/hold fields,
  are not checked on a pedal.
- The `.hlx` editor's amp and cab panel keeps the older single-column layout.
- Command Center changes use the whole-preset write (op 21), not the pedal's own live commands.

## 1.0.0-beta.2 - 2026-11-17

### New
- **Guitar input card** on a pedal preset's Controllers tab: the guitar input impedance (Auto to 1M) and the
  guitar pad (Global, Off, On). On the loaded preset a change reaches the pedal at once (kept by "Save on
  pedal"); on a file or a slot copy it changes the document. Auto lets the pedal pick the impedance from the
  first effect in the chain; a value set here is the preset's own and wins over the pedal's global setting.
- The file formats notes record how the preset and global input settings interact (checked on an HX Stomp).

## 1.0.0-beta.1 - 2026-11-17

First beta. Checked on an **HX Stomp** (through a Helrig Box for this release); the other units
(HX Stomp XL, HX Effects, Helix Floor / LT / Rack) are supported on a best-effort basis from their
documented layouts and panels and have not been tried on real hardware.

### New
- **Light theme**, or follow the computer (Settings > Display). Text contrast is tested for every
  surface and accent colour.
- **Compare two presets** (Open menu): open tabs or files, `.hlx` or pedal documents; lists blocks
  added, removed or swapped, on/off, knob values, split and merge points, tempo and snapshot names.
- **Restore the whole pedal** from a "Back up the whole pedal" folder, with a list of what it holds first;
  every slot it replaces is backed up and every preset is read back.
- **Command Center on the pedal document**: every command is shown; FS 4 and FS 5 can be set to HX Preset
  or HX Snapshot, in a file or live on the loaded preset (kept by "Save on pedal").
- **Snapshot copy on the live preset** through the pedal's own copy and paste.
- **Auto-reconnect** after a dropped link (USB or Helrig Box): five tries, a little later each time.
- Stereo sends, returns and FX loops show two jacks; jack hints on hover for the Helix Floor, Rack, LT,
  HX Stomp, Stomp XL and HX Effects.
- Input and output choices are listed per unit, and converting a preset between units keeps what the
  routes mean (and renames the input and output models between the Helix and Stomp families).
- Sending a `.hlx` to the pedal carries each path's input and output settings, the guitar input
  impedance and the guitar pad.
- A connect prompt in the empty pedal list; a plainer conversion dialog.

### Fixed
- Removing a split no longer puts parallel-lane effects beyond the unit's last place (or deletes them);
  it refuses with a note when the main lane is full.
- The Send / Return picker follows the unit of the open file, not the connected pedal; a return is only
  placed at or after the clicked line, and "before its send" is checked on both lanes.
- A split or merge imported from a longer path is held to the pedal's columns.
- Converting between units moves blocks past the new unit's last place and drops the leftovers of a
  removed Path 2; the HX Stomp XL no longer offers Ext Amp; the HX Effects has two FX loops, not four.
- A damaged `.hxb` can no longer crash the app; presets taken from backups get the usual `.hlx` wrapper;
  `.hlx` files saved with a byte-order mark open.
- Sending a `.hxpreset` made for another unit is refused; the editor's Send is off for a file of
  another unit; a write into the playing slot refreshes the live tab; a failed IR delete refreshes the list.
- The pedal model is read from its own name, not from a Helrig Box's label.
- A bypassed FX Loop keeps the dry line drawn.

### Known limits
- Only the HX Stomp has been tried on real hardware; slot copy / swap / move by drag, USB (not re-run for this beta), restore of IRs
  and several setlists have not.
- Command Center on the pedal: only FS 4 and FS 5, and only HX Preset and HX Snapshot, can be changed;
  other commands are shown and kept as they are (change them in HX Edit).
- The Helrig Box token is kept in plain text in the settings file, and the Box link is not encrypted.
- Windows only; the installer is not code-signed, so SmartScreen may warn.

## 0.3.0 - 2026-11-16

First tagged release.

### Safer for your presets and files
- Slot writes stop if a backup can't be made; IR imports and deletes back up the IRs they replace.
- Documents go to the pedal only through a checked encoder (no stale offset table, nothing
  unknown after the tone section); a write is read back and compared.
- Closing a tab or the window with unsaved changes asks first.
- Files are written atomically; imports never replace an existing file; a damaged settings file
  is kept as `settings.json.bad` instead of being overwritten.
- Fixes for USB recovery after a failed read or a timed-out write, IPv6 Helrig Box addresses,
  and a crash on damaged preset files.

### Usability
- Slot restore from backups, a full backup (all setlists and IRs), delete to the Recycle Bin.
- Drag and drop, recent files, reopen last session, F2 rename, shortcuts window (F1),
  keyboard shortcuts for tabs, name length counters, library search and sort.
- The window remembers its size and position; a Getting started card; automatic reconnect
  of a USB pedal; a Revert button for the live preset.
- The Classic look was removed; the accent colour is chosen in Settings.
- The playing preset opens in a live tab when you connect.

### Under the hood
- No compiler warnings, clippy clean, formatted with rustfmt, CI on every pull request.
- A fake pedal for testing the protocol layer without hardware.
- Windows installer and zip built by GitHub Actions.

### Notes
- ToneStrand needs Line 6's HX Edit installed on the same computer: it reads the effect names
  and knob ranges from it.
- The program is not code-signed, so Windows SmartScreen may warn on the first run.
