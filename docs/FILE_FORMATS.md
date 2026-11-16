# File and document formats

## 1. The pedal's preset document (`.hxpreset`)

What the pedal stores in a slot and sends for "fetch slot" / "read loaded preset". ToneStrand
saves it byte for byte as `.hxpreset`. Code: `devpreset.rs`, `msgpack.rs`.

```
"l6-helix\0"            MessagePack string
offset table (48 bytes) stored with the *string* type (str16)
tone                    one MessagePack map
```

- The offset table holds the tone's start, the offsets of top-level tone keys
  `0, 1, 3, 4, 2, 5, 6, 7, 10` (in that order), then the total length twice. It is rebuilt
  on every encode.
- `msgpack.rs` remembers the format byte each value was read with, so a document read and
  written back unchanged is byte-identical (checked on 126 real presets).

### Tone map keys

| Key | Meaning |
|---|---|
| `0`, `1` | path 0, path 1 (each a map; `22` = its slots) |
| `3` | footswitches (`{7: 2, 8: [per footswitch: nil or records]}`) |
| `4` | controllers (one entry per controller number) |
| `5` | settings (`16` = tempo, `30` = guitar input impedance: 0 Auto, 1 10k, 2 22k, 3 32k, 4 70k, 5 90k, 6 136k, 7 230k, 8 1M; `134` = guitar pad: 0 Global, 1 Off, 2 On). Same numbering as `.hlx` `@guitarinputZ` / `@guitarpad`. The pedal also has global settings under the same ids; a preset value is its own, pad "Global" defers to the pedal's. |
| `7` | meta |
| `10` | snapshot section (`6` = current snapshot, `10` = snapshot list) |
| `35` | firmware |

### Slots (`path[22]`)

A path of `c` columns (8 on HX Stomp/XL and Helix, 9 on HX Effects) has `2c + 4` slots:
input `0`, lane A `1..=c`, output `c+1`, split `c+2`, lane B from `c+3`, merge `2c+3`.
The slot's wire number (used by live requests) is `20 * path + slot`.

Each slot: `{19: kind, 20: body}`. Kind: `0` input, `1` output, `2` split, `3` merge,
`6`/`7` block, `8` empty.

Body keys: `24 → 25` model number (the model's position in HX Edit's `Helix.sym`), `26`
paired model (amp's cab, dual cab's second cab), `12` paired values, `10` enabled,
`11 → 4` knob values in `Helix.sym` parameter order, `7` input/output values, `15` split
record, `17` merge record.

Split and merge: their own record (`15` / `17` inside the body) holds the attach point at key
`13` (the main-lane slot they sit before); body key `18` is the "active" flag (the path is
split).

### Snapshots (`tone[10][10]`)

Each snapshot entry: `4` name, `5` tempo, `2` knob values for the controller assignments
(64 records `[footswitch on, assignment order t, value]`, unused `[false, 64, nil]`),
`3` per-slot on/off (indexed by wire number, element 1).

### Controllers (`tone[4]`)

One entry per controller number; each holds records
`{0: order t, 1: {0: controller, 1: cc (4 unless MIDI), 2: min, 3: max,
4: 0 knob / 1 setting / 2 on-off, 5: wire number, 6: {28: cab, 29: knob index, 41: trails},
7: 0, 13: snapshot-disable}}`, ordered by controller, then slot, then knob.

### Footswitches (`tone[3]`)

`{7: 2, 8: [...]}`; a record `{10: order, 11: {0: 1 block on/off / 2 knob, 5: label,
6: LED colour, 7: on, 8: wire number, 2: 0, 9: knob target}, 12: momentary, 13/14: custom
label, 15/16: custom colour}`.

### Command Center (`tone[2]`)

`{0: [command id per slot], 1: [[id, arg1 .. arg6] per slot]}`, one slot per trigger. The HX Stomp has 13,
in this order: Instant 1-6, FS 4, FS 5, FS 1-3, EXP 1, EXP 2 (FS 4 and FS 5 are confirmed from real
presets; the rest follow the order of the Command Center rows). The ids are the ones `.hlx` files
use (14 HX Preset, 15 HX Snapshot, 16/17 HX Looper, 18 MIDI CC ...) and the arguments follow the
command's fields in order: HX Preset `[preset]` (0 Next, 1 Prev, 2+ the slot), HX Snapshot
`[behaviour, press, release/hold]`. Only those two are confirmed; other commands' argument order is
not. `snapshot[n]` entry 1 holds the per-snapshot command values `[13, false, [7 values]]` per slot (all
defaults in the presets read). Written to a slot, the pedal keeps them as sent.

### `.hlx` → pedal document

`DevicePreset::from_hlx` starts from a document read from the same pedal (so the firmware
shape is right), empties both lanes and places the `.hlx` blocks: a block goes to slot
`(@path == 1 ? c+3 : 1) + @position`; split/merge attach = `@position + 1`; knob values in
`Helix.sym` order with each knob's type; on/off per snapshot; controllers, per-snapshot
values and footswitches as above.

## 2. `.hlx` (JSON)

The preset format Helix editors export. Code: `hlx.rs`, `snapshots.rs`, `controllers.rs`,
`units.rs`.

- `data.device`: the unit's device number (`2162694` HX Stomp, `2162699` HX Stomp XL,
  `2162693` HX Effects, `2162689` Helix Floor, `2162692` Helix LT, `2162690` Helix Rack).
- `data.tone.dspN`: `inputA`, `outputA`, `split`, `join`, `blockN` (`@model`, `@position`,
  `@path`, `@enabled`, `@stereo`, knob values), `cabN` riding with an amp or a dual cab
  (referenced by the block's `@cab`).
- `data.tone.global`: `@tempo`, `@current_snapshot`, `@topologyN` (`"A"` single lane,
  other values split).
- `data.tone.snapshotN`: `@name`, `@valid`, `blocks/dspX/blockN` (on/off),
  `controllers/dspX/blockN/Knob/@value`, `commands`.
- `data.tone.controller/dspX/blockN/Knob`: `@controller`, `@min`, `@max`, `@cc`.
- `data.tone.footswitch/dspX/blockN`: `@fs_index`, `@fs_momentary`, `@fs_label`, ...
- Command Center: `data.tone.command<Slot>` (`@command` 1..20) and per-snapshot values in
  `snapshotN/commands/command<Slot>`.

HX Edit files may contain `split`/`join` even when the path is a single lane (topology `"A"`,
nothing on the lower lane); ToneStrand treats those as not split.

## 3. HX Edit backups and setlists

Code: `backup.rs`.

- `.hxb`: `"AF6L"`, u32 version 1, u64 index offset, u64 entry count, u32 device, ...;
  36-byte index entries: 4-character tag stored backwards, u64 offset, u64 stored size,
  u32 compressed flag, u64 size. Tags: `SLnn` setlists (zlib JSON
  `{data: {meta: {name}, presets: [...]}}`), `SLNM` setlist names, `Innn` IRs (`.wav`).
- `.hls`: JSON `{schema: "L6Setlist", encoded_data: base64(zlib), compression: {type,
  decompressed_size, crc32}, meta: {name}}`.

## 4. User IRs

Code: `irs.rs`.

- On the pedal: 2,048 samples, 32-bit float little-endian, 48 kHz mono (8,192 bytes).
- Reading a `.wav`: PCM 8/16/24/32-bit or float 32/64. 48 kHz mono float goes over byte for
  byte; anything else is mixed to mono, resampled (windowed sinc) and cut or padded to
  2,048 samples.
- A converted NAM capture is recognised by its first samples: `0.5, -0.5, 3.0`.

## 5. NAM A2 Lite captures

Code: `nam.rs`. For the community firmware
[hx-stomp-nam-lite](https://github.com/nocaseguy/hx-stomp-nam-lite) (HX Stomp, HX Stomp XL,
HX Effects; firmware 3.80). The block takes the place of the mono Xenomorph Fuzz model
(`HD2_DistXenomorphFuzz`).

- Knob mapping (stock parameter → NAM): Gain → Input (-20..+20 dB), Tone → Treble (±12 dB at
  2.5 kHz), Level → Output, Clipping → Polarity (0 normal, 1 inverted), OscLevel → Bass
  (±12 dB, 150 Hz shelf), OscTone → Mid (±12 dB), MinFreq → Capture (User IR slot 1..128,
  otherwise empty), MaxFreq → Mid Freq (150..3000 Hz), WaveShape → Q (0.7 / 1.4 / 2.8),
  Sensitivity → High Cut (1.0 = off).
- At most 3 NAM blocks per preset; about 23% of an HX Stomp's DSP each.
- Converter output: a 2,048-sample, 48 kHz mono float32 `.wav`: header `0.5, -0.5, 3, 2^e,
  1871/1024, level, loudness, 0`; then the capture's 1,871 weights in the block's load order;
  then 72 words of its silence state (each / 16); everything divided by `2^e` so every sample
  stays within ±15.5; the rest 0. Level word `10^(-dBu/20)` (input level, 11.5 dBu reference),
  loudness word `10^((-18 - loudness)/20)` (0 when it isn't below 15.8489). Captures not at
  48 kHz, or not the A2 Lite shape, are refused.
- `tests/data/` holds a real capture and the reference outputs of the project's own
  `nam2ir.py` (with and without `--no-level --no-loudness`); the test
  `matches_nam2ir_py_byte_for_byte` checks the converter against them.
- Without the firmware the pedal refuses these files and the transfer can hang; the upload
  window asks the person to confirm they have it.
