# Roadmap

What is left after 1.0.0-beta.3, in the order I would do it. "Needs a unit" means it cannot be finished without
someone running it on that hardware; everything else can be built and tested from the repository.

## 1. Command Center on the other units

**State:** the grid, the editor, the footswitch records and the `.hlx` conversion now follow a per-unit slot table
(`controllers::device_slots`): HX Stomp 13 slots (checked on a pedal), HX Stomp XL 16, HX Effects 14, Helix 24
(guessed from the documentation). Those units show a note that the layout is not checked.

**Steps**
1. *(Done)* Per-unit slot table, grid rows shared with the `.hlx` tab, test that every unit's slots match its
   document size, no repeated keys, no slot on two rows.
2. *Needs a unit, 30 minutes per unit.* A tester sets one command on the first and the last footswitch, on an
   EXP pedal and on an instant, with the other editor connected through `--box-proxy`, then sends the `box-proxy.log`
   and the output of `--dump-loaded` and `--commands`. See "What a tester does" below.
3. From that file: confirm or fix the slot order, the footswitch-table index of each switch, the keys for the Helix
   extras (`commandExpSwitch`, `commandVariaxVol`, `commandVariaxTone` are guesses), and the per-snapshot record
   count (13, 16, 14 and 23 in the notes).
4. Drop the "not checked" note for that unit (`controllers::layout_verified`) and record the result in
   `docs/COMMAND_CENTER.md`.

**Done when:** a command set in the editor on a Stomp XL, HX Effects and Helix reads back the same from the unit
after "Save on pedal", and a `.hlx` from that unit converts without a note.

## 2. The `.hlx` editor's amp and cab panel (no unit) — done

The pedal editor draws an amp with a cab as two sections (photo, divider, knobs; a line between). The `.hlx` editor
keeps the older single column. Move the knob code into the same `body_section` helper (`editors/mod.rs`).
Risk: that code also normalises values and handles controller menus, so do it as its own change with the
`hlx_add_move_remove` and compare tests running.

## 3. Live Command Center edits with the pedal's own requests — done, checked on the Stomp

Ops 67, 68 and 88 are captured (`docs/PROTOCOL.md`). Today every change rewrites the whole edit buffer (op 21).
The live requests change one value, avoid overwriting a knob turned on the pedal at the same moment, and are quicker.

**Steps:** a `Cmd::SetCommandLive` that sends 67 / 48 / 33 for a type change and 68 for a value (selecting the
snapshot with 88 first when it is not the pedal's current one), keep op 21 as the fallback; verify with
`--dump-loaded` after a rapid series of edits; compare timing.

## 4. Preset library search and tags — done (tags, search in snapshot names)

Find a preset by effect, amp, cab, tempo or a tag across the library and the backups. The data is already parsed
(`library.rs`, `compare.rs` summaries).

**Steps:** an index built when the library is scanned (name, blocks, snapshot names, tags in a sidecar file), a
search box on the Library page, tag editing, "open" and "send to pedal" from the results.

## 5. Problem report bundle — done

A button in the Activity panel that saves the log, the app and unit information and, on request, the loaded preset
into one `.zip` the person can attach to an issue.

**Steps:** collect the log lines already kept for the Activity panel, the version and unit, the catalog status and
(optional, with a tick box) the live document; write to the Downloads folder; show the path. No network.

## 6. First-run check — done

On the first connection, say which unit was found, which parts of the app are checked on that unit and which are not
(this file, section 1, is the source), and how to report a problem (item 5).

## 7. Housekeeping — help grouped, portable CI job added; the rest open

- Move the write-to-pedal test tools (`--cc-raw`, `--cc-snap`, `--send-hlx`, `--put-slot`) behind one
  `--lab` prefix, or mark them clearly in `--help`.
- Bank/Program and HX Looper switch names and defaults, and HX Looper's press and release/hold fields: one capture
  on the Stomp.
- macOS and Linux builds: a CI job that builds them; running them needs a person with the machine.

## What a tester does for item 1 (about 30 minutes per unit)

1. Install this app and a ToneStrand Box on the unit (or connect by USB if the other editor is not used).
2. Start the proxy: `tonestrand --box-proxy 127.0.0.1:80 <box address> <folder>`, point the other editor at
   `127.0.0.1`, and wait for it to show Connected.
3. In the other editor, on a spare preset, set: Instant 1 = MIDI CC with distinct values; the first footswitch = CC
   Toggle with distinct values; the last footswitch = Note On; an EXP pedal = MIDI CC; and, on a Helix, the EXP toe
   switch and a Variax slot if they are offered. Write down which trigger got which values.
4. Stop the proxy, wait five seconds, and run `tonestrand --dump-loaded <box address> <folder>\after.hxpreset`
   and `tonestrand --commands <folder>\after.hxpreset`.
5. Send the folder (`box-proxy.log`, `after.hxpreset`) and the notes from step 3.

## Status of item 3 (live Command Center requests)

Checked on an HX Stomp (slot 42B, edit buffer only). `device::change_command_center` sends the pedal's own
requests and reads the preset back; the slot's command, its values and its Momentary flag must be what the edit should
have made, else the edit buffer is written. Findings from the check: the pedal keeps the live values in the command
itself and writes a snapshot's own copy later; it makes a snapshot's records only for slots it touched; a CC Toggle's
sixth value is a running state the pedal sets as it likes; a Momentary request on a Note On changes its note-off
value. An edit for a snapshot the pedal has not selected is always written to the edit buffer (the pedal's own
request would recall that snapshot). Lab tool: `--cc-live` (with `TONESTRAND_DEBUG_CC=1` it says why it fell back).
