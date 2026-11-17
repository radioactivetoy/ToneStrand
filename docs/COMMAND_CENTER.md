# Command Center: how presets store it and how to edit it live

A brief for building ToneStrand's Command Center support on pedal documents (presets read from
or written to the pedal) and live on the loaded preset. `.hlx` editing already exists
(`controllers.rs`, Command Center tab).

Sources: an existing editor's request builders and document writer (read, not copied), checked
against real HX Stomp presets. **Status:** the document layout matches 126 real presets. The live
requests in section 6 were then captured from that editor's traffic to a pedal (through the Box with
`--box-proxy`), which fixed the slot order, the field positions and the argument layouts of ten
command types. Types 4, 6, 12 and 16 are still unconfirmed (marked in section 2).

## 1. Slots (one per trigger)

Every preset has a fixed list of Command Center slots. The slot's position in the list is the
slot number used in the document and in live requests.

| Unit | Slots | Order (slot 0 first) |
|---|---|---|
| HX Stomp | 13 | Inst1-6, FS1-5, ExpPedal1, ExpPedal2 |
| HX Stomp XL | 16 | Inst1-6, FS1-8, ExpPedal1, ExpPedal2 |
| HX Effects | 14 | Inst1-6, FS1-6, ExpPedal1, ExpPedal2 |
| Helix Floor / LT / Rack | 24 | Inst1-6, FS1-12, ExpSwitch, ExpPedal1-3, VariaxVol, VariaxTone |

Rule for the non-Helix units: 6 instant slots, then `slots - 8` footswitches, then 2 pedals.

The `FSn` names are Command Center switch numbers. **Live capture (HX Stomp): FS 1 to FS 5 are slots 6
to 10, in that order**, and Instant 1-6 are slots 0-5, EXP 1 and EXP 2 slots 11 and 12. The other editor
draws FS 4 and FS 5 in the top row beside the instants and FS 1-3 in the bottom row with the pedals,
but the slot numbers are as listed. An earlier version of this brief said slots 6 and 7 are the pedal's
FS 4 / FS 5; that is superseded: the presets whose HX Preset commands sit in slots 6 and 7 have them on
the Command Center's FS 1 and FS 2. When a switch is used by Command Center, the editor also sends the
footswitch index (section 6), so the pedal knows which physical switch it is.

`.hlx` keys are `command` + slot name: `commandInst1`, `commandFS2`, `commandExpPedal1`, ... Confirmed with a Stomp preset exported by the other editor ("Wish acoustic.hlx"): `commandFS1` to `commandFS5` are slots 6 to 10 and only `commandExpPedal1` is written (no `commandExpPedal2`). That file keeps a wait of 5000 where the pedal clamps to 20, so clamp when converting.

## 2. Command types

`@command` / type id, its name, what triggers it, and its six argument fields **in order**. `—` = unused
(written as 0). Type 12's fifth field is the constant 1.

| Id | Name | Kind | Arg 1 | Arg 2 | Arg 3 | Arg 4 | Arg 5 | Arg 6 |
|---|---|---|---|---|---|---|---|---|
| 0 | (none) | — | — | — | — | — | — | — |
| 1 | MIDI CC | switch | channel | cc | value | — | — | — |
| 2 | CC Toggle | switch | channel | cc | initial (Dim) | toggle (Lit) | — | current |
| 3 | MIDI CC (pedal) | pedal | channel | cc | min | max | — | — |
| 4 | Bank/Program | switch | channel | bank0 (MSB) | bank32 (LSB) | program | — | — |
| 5 | Note On | switch | channel | note | velocity | noteoff | — | — |
| 6 | MMC | switch | message | — | — | — | — | — |
| 7 | Ext Amp | switch | select (jack) | — | — | — | — | — |
| 8 | Ext Amp | instant | select (jack) | waittime | — | — | — | — |
| 12 | Hotkey | switch | modifier1 | modifier2 | modifier3 | keystroke | 1 | — |
| 13 | Hotkey | instant | modifier1 | modifier2 | modifier3 | keystroke | waittime | — |
| 14 | HX Preset | switch | preset | — | — | — | — | — |
| 15 | HX Snapshot | switch | behavior | press | relhold (release/hold) | — | — | — |
| 16 | HX Looper | switch | behavior | press | relhold | — | — | — |
| 17 | HX Looper | instant | function | — | — | — | — | — |
| 18 | MIDI CC | instant | channel | cc | value | waittime | — | — |
| 19 | Bank/Program | instant | channel | bank0 | bank32 | program | — | — |
| 20 | MMC | instant | message | — | — | — | — | — |

**Checked on a pedal** (set from the other editor, read back from the document): 1, 2, 3, 5, 13, 14,
15, 17, 18, 19, 20; the arguments are in the order of the table. **Not checked: 4, 6, 12, 16** (the
switch versions of Bank/Program, MMC, Hotkey and HX Looper; assumed to follow their instant
versions). ToneStrand marks them "(not checked on a pedal)".

What the pedal puts in a command's arguments when its type is set (seen in its change events; the rest
start at 0): CC Toggle and pedal MIDI CC `[0, 0, 0, 127]`, Note On `[0, 48, 100, 1]`, HX Snapshot
`[0, 1, 0]`, Looper instant `[1]`. Changing the type resets the fields. Each snapshot keeps its own copy
of a command's values (section 4).

Which types a slot may use:

- **FS slots (and ExpSwitch):** 1, 2, 4, 5, 6, 7, 12, 14, 15, 16
- **Inst slots:** 18, 19, 20, 8, 13, 17
- **ExpPedal slots:** 3
- The **HX Stomp** has no Ext Amp: leave out 7 and 8.

### Field values

| Field | Values |
|---|---|
| channel | 0 = "Base" (the pedal's MIDI base channel), 1-16 |
| cc, value, min, max, program, bank0, bank32, velocity, initial, toggle | 0-127 |
| waittime | 0-20 (the pedal clamps anything higher to 20; the editor's own spec says 5000) |
| note | 0-127 note names (HelixControls display type `note`) |
| noteoff | display type `note_off` (2 choices) |
| message (MMC) | display type `mmc_f15_280` (8 choices) |
| select (Ext Amp) | `ext_amp_select` (switch, 3) / `ext_amp_instant_select` (instant, 4); HX Effects uses the `_f15` variants |
| modifier1-3 | `key_modifier_mac` (5 choices) |
| keystroke | 0 "(None)", A-Z, 1-0, Enter, Esc, Backspace, Tab, Space, punctuation, Caps Lock, F1-F12, navigation keys, keypad keys, App, Power (USB HID order) |
| preset (HX Preset) | 0 Next, 1 Previous, 2+ = slot (preset index + 2); 126 slots on HX Stomp, 128 elsewhere |
| behavior (HX Snapshot / Looper) | `fsBehavior` (2 choices) |
| press, relhold (HX Snapshot) | `snapshotFunction_P33` on HX Stomp, `_P36` on Stomp XL, `_F15` on HX Effects, `snapshotFunction` on Helix (11 choices, e.g. Next/Previous/Snapshot 1..n). Show the preset's own snapshot names where an option is "SNAPSHOT n" |
| press, relhold (HX Looper) | `looperFunction` (15 choices) |
| function (Looper instant) | `looperFunctionInst` (14 choices) |

The named choices come from HX Edit's `HelixControls.json` (the same file `catalog.rs` reads
display types from). Read them through the catalog; don't hard-code them.

`.hlx` files may carry fields a type doesn't have. Types 1 and 2 can hold leftovers such as
`@initial`/`@toggle`/`@current`/`@min`/`@max` on a type 1, or `@value`/`@min`/`@max` on a type 2. These are
dropped when writing to the pedal, as HX Edit does. Tell the person if a non-zero one is dropped.

## 3. In the pedal document: tone key `2`

```
tone[2] = {
  0: [ id_slot0, id_slot1, ... ]                    // i16 per slot: the command type (0 = none)
  1: [ [id, a1, a2, a3, a4, a5, a6], ... ]          // 7 × u8 per slot, args in the order above
}
```

- Both lists have exactly the unit's slot count (13 / 16 / 14 / 24).
- An unused slot: id `0` and `[0, 0, 0, 0, 0, 0, 0]`.
- Each argument is written as `value & 255`. For a slot with type `t`, the args are the type's
  fields in table order; a constant in the table (type 12's `1`) is written as-is.
- Keep each value's MessagePack format when editing an existing document (`msgpack.rs` does this),
  so an unchanged document stays byte-identical.

The footswitch's label, LED colour and "primary" flag (`@fs_label`, `@fs_ledcolor`,
`@fs_primary`, `@fs_momentary`, `@fs_customlabel`, `@fs_customcolor`) aren't stored here; they go
in the footswitch table (`tone[3]`, see FILE_FORMATS.md).

## 4. Per-snapshot values

Some commands can take different values per snapshot (for example an HX Snapshot switch that
calls snapshot 2 from snapshot 1 and snapshot 3 from snapshot 2). Each snapshot entry's field `1`
holds these records:

```
snapshot[n][1] = [
  [slot, false, [0, a1, a2, a3, a4, a5, a6]],    // one per Command Center slot in use (sorted by slot)
  ...
  [SLOTS, false, [0, 0, 0, 0, 0, 0, 0]],         // padding up to the unit's record count
]
```

- One record per used slot, **except HX Preset (type 14), which is never per-snapshot**.
- The record's args are the slot's base settings, overridden by that snapshot's values; they use
  the same field order as the slot's type. The first element of the inner list is `0` (not the type).
- The second element was `false` in every preset seen. Write `false`.
- Padding records use the slot count as their slot number (13 on HX Stomp).
- Record counts per unit: **13** HX Stomp, **16** Stomp XL, **14** HX Effects, **23** Helix.
- In `.hlx`: `snapshotN.commands.command<Slot> = { "@press": .., "@relhold": .., "@behavior": ..,
  "@fs_enabled": false }`, merged over the base `command<Slot>`.

## 5. Building a preset (`.hlx` → pedal document)

1. Get the slot list for the unit (section 1). For each `command<Slot>` in the `.hlx`, find its slot
   index; refuse a key the unit doesn't have (e.g. `commandFS8` on an HX Stomp), saying why.
2. Refuse a type outside the table, and a set field the type has no place for (e.g. `@waittime` on
   a type 1), unless it's one of the known leftovers in section 2, which are dropped with a note.
3. Fill `tone[2][0]` with the types and `tone[2][1]` with `[type, args...]` (section 3).
4. For every snapshot, build field `1` as in section 4.
5. Put each Command Center footswitch's label/colour into the footswitch table as for bypass
   switches.
6. Encode, re-parse, compare. Write to a slot only after backing the slot up (`device::backup_slot`).

## 6. Editing live (the loaded preset)

All on the data channel, service 6, keys: `24` slot, `77` command type, `28` field position,
`81` value.

| Op | Name | Args | Use |
|---|---|---|---|
| 67 | set command type | `{24: slot, 77: type}` | change a slot's type (0 clears it) |
| 48 | command info | `{24: slot}` | read the slot back after changing its type |
| 68 | set command value | `{24: slot, 77: type, 28: position, 81: value}` | set one field; `position` = the field's **1-based** argument number (confirmed: channel is 1, CC is 2 on a MIDI CC) |
| 33 | footswitch index | `{102: fs}` | after setting a type on an FS slot, tells the pedal which physical switch it is (1-based) |
| 58 | latching | `{102: fs, 65: bool}` | the Latching toggle of an FS slot (`fs` 0-based). Only **CC Toggle (type 2)** has it; MIDI CC and the rest have none. The other editor calls it "Momentary" and writes it to `.hlx` as `@fs_momentary` (momentary = not latching); the pedal keeps it in the footswitch table (probably field 12), polarity of op 58 not yet confirmed |
| 88 | select snapshot | `{92: n}` | values are per snapshot, so select the snapshot first |

The pedal answers a type change with event 35 and a value change with event 36, which carries the old value
(key 80). Events 37, 42, 46 and 49 also appear while editing.

The editor this comes from runs them in this order:

- **Type change:** op 67, then (unless type 0) op 48, then op 33 with the footswitch number if
  the slot is a footswitch. Send them one after another, waiting for each reply.
- **Value change:** op 68 with the field's 1-based argument number. A field the type doesn't
  have is refused before anything is sent.
- **Per-snapshot values:** the pedal keeps its own copy per snapshot (seen in the capture). Most likely:
  select the snapshot (op 88), then set the value with op 68. Not yet tried by ToneStrand; use the op 21
  fallback until it is.
- **Fallback (what ToneStrand does today, tried on an HX Stomp):** change the document and write the whole edit buffer with op 21
  (`{110: document}`, see PROTOCOL.md). ToneStrand does not use ops 67/68 yet.
- Either way, "Save on pedal" (op 71) stores it; reloading the preset drops unsaved changes.

## 7. Suggested UI

- A Command Center tab in the Controllers panel for pedal documents, like the `.hlx` one:
  rows per slot (Instant 1-6, the footswitches by their pedal labels, EXP pedals), a type
  chooser limited to that slot's kind, and the fields of the chosen type with their choices.
- For HX Snapshot / Looper on a footswitch, a per-snapshot value row (one value per snapshot, as
  the Snapshots tab does for knobs).
- Show footswitch slots with the Command Center's own numbers (slot 6 = FS 1 ... slot 10 = FS 5).
  ToneStrand's grid does: the top row Instant 1-6 plus FS 4 and FS 5, the bottom row FS 1-3 and EXP 1-2,
  with a snapshot badge and a detail card per trigger.

## 8. How to check it

- Round trip: read every backed-up preset, decode `tone[2]` and the snapshot command records,
  re-encode, compare bytes (extend `--ctl-check` or add `--cmd-check`).
- Live: on a spare preset, set a type and a value with ops 67/68, read the document back
  (op 22) and compare `tone[2]` with what you expected; then reload the preset to discard.
