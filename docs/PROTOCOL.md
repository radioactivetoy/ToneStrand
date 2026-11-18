# Protocol

How ToneStrand talks to a Helix-family pedal. The same byte stream goes over USB directly or
through a ToneStrand Box, which relays USB bulk transfers over a WebSocket.

Values marked *(estimated)* work in practice or follow the general pattern, but haven't been
confirmed in every case.

## 1. Transport

### USB

| | |
|---|---|
| Vendor | `0x0e41` (Line 6) |
| Interface | 0 |
| Endpoints | bulk OUT `0x01`, bulk IN `0x81` |
| Transfer | one bulk transfer carries one or more frames (section 2) |

Implemented in `wire.rs` (`UsbWire`, pure-Rust `nusb`).

### ToneStrand Box (and the Helrig Box)

The box is a DIY build: firmware, browser flasher and guides at https://github.com/radioactivetoy/tonestrand-box.
It is a fork of the [Helrig Box](https://github.com/maveric1977/helrig-box) (maveric1977), and its firmware is the
open-source ToneRelay Wi-Fi relay by Mark Peskett and maveric1977 (https://github.com/maveric1977/ToneRelay). This
section follows their relay protocol (`docs/raw-relay-protocol.md` in the box repository); credit for the protocol
is theirs. ToneStrand talks to a ToneStrand Box, a Helrig Box or a plain ToneRelay build the same way.

| Rule | Value |
|---|---|
| Discovery | mDNS service types `_tonestrandbox._tcp.local.`, `_helrigbox._tcp.local.` and `_tonerelay._tcp.local.`; fallback names `tonestrandbox.local`, `helrigbox.local`, `tonerelay.local`, looked up again every few seconds while a box does not answer (a box that moved to another network is followed to its new address, matched by its id) |
| Status | `GET http://<box>/api/relay` → JSON (box id, pedal attached, relay mode, client connected, `direct`: the box is in direct mode, on its own network at `192.168.4.1`) |
| Pairing | `POST /api/pair` → nonce, 30 s window; the person presses BOOT on the box; poll `GET /api/pair?nonce=<n>` until it returns `token` (32 hex characters) |
| Relay mode | `POST /api/relay {"on": true}` with the token |
| Relay link | WebSocket `ws://<box>/raw` (port 80), header `Authorization: Bearer <token>` on the upgrade |
| Hello | the box's first text frame `{"t":"hello","proto":0,...}`; refuse other `proto`; `pid: null` means no pedal |
| Frames | one binary WebSocket frame = one USB bulk transfer (OUT or IN), at most 4,096 bytes |
| Errors | text frames `{"t":"error","code":...}` (e.g. `usb_error` = the box lost USB contact with the pedal, `relay_unavailable`) |
| Liveness | the client must send something every 5 s (the box drops it after 6 s); ToneStrand sends `{"t":"ping"}` after 3 s of quiet |

Direct mode (the box's own network, `192.168.4.1`, entered by holding BOOT for 2 to 4 seconds) uses the same protocol on that address.
The box's Wi-Fi setup commands (`/api/cmd`) are not part of what ToneStrand uses and are not protected by the pairing token.

Implemented in `wire.rs` (`BoxWire`) and `boxes.rs`. Large transfers through the box are paced
more gently than over USB (see 3.4).

## 2. Frames

Every bulk transfer holds frames, each padded to a multiple of 4 bytes:

| Bytes | Field |
|---|---|
| 0-2 | payload length, u24 little-endian |
| 3 | flags: `0x18` data, `0x28` handshake |
| 4-5 | destination node, u16 LE |
| 6-7 | source node, u16 LE |
| 8.. | payload (a channel message) |

Lengths above 8,192 mean the stream is out of step; the rest of the transfer is dropped.

## 3. Channels

Three channels, each a pair of nodes (device / host):

| Channel | Device node | Host node | Use |
|---|---|---|---|
| Control | `0x1001` | `0x03ef` | opened with service 5, then closed |
| Events | `0x1002` | `0x03f0` | notifications from the pedal (service 4) |
| Data | `0x1080` | `0x03ed` | requests and replies (service 6) |

### 3.1 Channel header (8 bytes, at the start of each payload)

| Bytes | Field |
|---|---|
| 0-1 | sequence, u16 big-endian |
| 2-3 | type, u16 BE: `2` hello, `0x04` data flag, `0x08` ack, `0x10` ping |
| 4-7 | ack, u32 LE |

- The ack value is `0x1000 + stream bytes received` on that channel. Acknowledge every data
  packet.
- **Pings (type `0x10`) must be acknowledged**, or the pedal stops replying on that channel.
- Handshake: a hello (type 2) whose ack slot holds `0x21000100`, followed by the four bytes
  `00 10 00 00`; after it the sequence jumps from 0 to 2.
- Open order: control (service 5, then closed), data (service 6), events (service 4).

### 3.2 Stream

Inside a channel the payloads form a byte stream of messages:

| Bytes | Field |
|---|---|
| 0-1 | origin, u16 LE (`1` = computer) |
| 2-3 | service, u16 LE |
| 4-7 | length, u32 LE |
| 8.. | MessagePack body |

### 3.3 Requests and replies (MessagePack maps)

- Request: `{102: txn, 100: opcode, 101: args}`; `txn` starts at 1000 per channel and
  increments.
- Reply: `{102: txn, 103: status, 104: result}`; status `255` = refused, with a code at key `111`.
- Notification (events channel): an event number and arguments. Events `1` and `20` mean the
  loaded preset changed on the pedal.
- Services: browse/storage operations (setlists, presets, fetch/write/rename/clear slots, IRs,
  browse open, begin) use **service 2**; live edit-buffer operations use **service 6**; all on
  the data channel.

### 3.4 Sending large messages

- 256 stream bytes per frame.
- Between chunks keep reading and acknowledging for 80 ms over USB, 240 ms through a
  ToneStrand Box.
- Before a write through the box, wait 400 ms after BEGIN.
- After any flash write, rename, clear or save, wait 750 ms before the next request.
- A transfer that fails half way leaves the pedal mid-transfer; send nothing more and ask the
  person to power-cycle it. More writes into a pedal in that state can leave it stuck in
  "transferring".

## 4. Opcodes

Keys used in arguments: `107` setlist, `108` preset index, `109` name, `110` document/audio,
`98` block (slot wire number `20 * path + slot`), `26` 0 block / 1 its cab, `28` knob index,
`119` value, `92` snapshot, `112` IR slot.

### Browse and storage (service 2)

| Op | Name | Args |
|---|---|---|
| 0 | list setlists | — |
| 1 | list presets | `{107, 101: 2}` |
| 4 | fetch slot | `{107, 108, 101: 2}` → document bytes |
| 6 | rename preset | `{107, 108, 109}` |
| 8 | write slot | `{107, 108, 109 name (16), 123: false, 124: false, 125: 0, 110: document as str16}` |
| 16 | clear slot | `{107, 108}` |
| 254 | browse open | `{}` |
| 255 | begin (before writes) | `{}` |

### Presets and live editing (service 6)

| Op | Name | Args |
|---|---|---|
| 20 | select preset | `{107, 108}`; done when op 23 reports the index (poll every 150 ms, give up after 10 s) |
| 22 | read loaded preset | → document bytes |
| 23 | preset info | → setlist, index, name |
| 28 | clear block | `{98}` (after op 78) |
| 30 | set parameter | `{98, 29: true, 26, 28, 119}` |
| 39 | add block | |
| 40 | set model | `{98, 100: {23 paired, 25 model, 26 cab or -1}}` (after op 78) |
| 41 | enable | `{98, 59: bool}` |
| 42 | route (input/output) | |
| 43 | move block | from / to wire numbers |
| 46 | move split | `{98: 20*path + attach}` |
| 47 | move merge | same |
| 21 | write the edit buffer | `{110: document}` (reply status 1; nothing is saved to a slot) |
| 71 | save | `{107, 108, 109}` |
| 78 | select block | `{98}` |
| 88 | select snapshot | `{92}` |
| 89 | rename snapshot | `{92, 109}` |
| 90 | copy snapshot | `{92}` (ignored: copies the *current* snapshot) |
| 91 | paste snapshot | `{92}` (ignored: pastes into the *current* snapshot) |

Op 21 (tried on an HX Stomp) replaces the loaded preset in the pedal's edit buffer with a whole document, so
anything that has no operation of its own (Command Center, the guitar pad and impedance) can be changed live:
read the current document, change it, write it back. The stored slot stays as it was until "Save on pedal";
reloading the preset drops the change. Found in TonePush's notes (MIT) and checked on the pedal.

Ops 90 and 91 (tried on an HX Stomp): copy takes the current snapshot, paste writes what was copied
into the current snapshot, so select the source, copy, select the destination, paste. The paste
carries the snapshot's name, block on/offs and knob values; only the loaded preset's edit buffer
changes (the stored slot stays as it was until saved). Renaming a snapshot to an empty name gives "New Preset".
Split and merge: the pedal refuses a split that would sit after the merge, even for a moment;
move whichever one keeps the split before the merge first.

### Command Center (live; captured from another editor's traffic, not sent by ToneStrand yet)

| Op | Name | Args |
|---|---|---|
| 67 | set command type | `{24: slot, 77: type}` |
| 48 | command info | `{24: slot}` |
| 68 | set command value | `{24: slot, 77: type, 28: argument number (1-based), 81: value}` |
| 33 | footswitch index | `{102: fs}` (1-based) |
| 58 | latching | `{102: fs (0-based), 65: bool}` |

The pedal replies with events 35 (type changed) and 36 (value changed, old value in key 80). The wait is
clamped to 20. ToneStrand changes the Command Center with op 21 for now.

See [COMMAND_CENTER.md](COMMAND_CENTER.md) for slot numbers, types and field positions.

### Controllers and footswitches (live)

| Op | Name | Args |
|---|---|---|
| 37 | assign controller | `{98, 29: true, 26, 28, 74 controller, 71 cc, 129: false}`; unassign = controller 0, cc 0 |
| 64 | controller CC | `{98, 29, 26, 28, 71 cc}` |
| 65 / 66 | controller min / max | `{98, 29, 26, 28, 119 value}` |
| 56 then 33 | assign footswitch (block on/off) | `{98, 102: fs-1}` then `{102: fs}` |
| 57 | unassign footswitch | `{98, 102: fs-1}` |

Controller numbers depend on the unit (see `snapshots::Family`): EXP 1/2 = 1/2, footswitches
from `fs_first`, snapshots and MIDI have their own numbers per family.

### User IRs (service 2, after browse open + list setlists)

| Op | Name | Args |
|---|---|---|
| 13 | list IRs | `{101: 2}` → `[{112 slot, 109 name, 104 id, ...}]` |
| 12 | IR info | `{112}` |
| 11 | IR audio | `{112, 101: 2}` → 8,192 bytes (2,048 float32 LE at 48 kHz) |
| 9 | import IR | `{112 slot, 113 checksum (u32: sum of the audio's LE u32 words), 109 name (≤26), 114: 1, 115: 3, 123: false, 124: false, 125: 0, 110 audio as str16}` (after BEGIN) |
| 10 | rename IR | `{112, 109}` |
| 15 | delete IR | `{112}` |

Slots are 0..127 on the wire, shown 1..128.

## 5. Slot labels

Preset index → bank `index / 3 + 1` and letter A/B/C (`proto::slot_label`).
