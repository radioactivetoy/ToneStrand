# Changelog

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
