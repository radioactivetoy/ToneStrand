# Plan: our own box (ESP32-P4), with Wi-Fi, Bluetooth MIDI and Bluetooth editing

Goal: a box that carries the pedal's USB to ToneStrand (and to Helrig) over Wi-Fi **and** over Bluetooth, keeps its
Bluetooth MIDI for setlist apps, and has the three known board problems fixed. It starts as a fork of
`maveric1977/helrig-box` (MIT) and its firmware `maveric1977/ToneRelay` (MIT, from `mpeskett88/ToneRelay`).

Read first (done): `helrig-box` issue 6 (the three board problems), `docs/dev/build.md`, `docs/dev/releasing.md`,
`ToneRelay/docs/raw-relay-protocol.md`, `esp32-p4/components/tonerelay/{wifi.c,ble.c,relay.c}`.

## What is known

- The relay (`/raw`, one client, bearer token, `mps` 512) is served by the box's web server. The box starts its own
  Wi-Fi network (`wifi.c`, mode APSTA at 192.168.4.1) whenever it cannot join a known network, and leaves it when it
  joins one. The relay should be reachable on it; not tried.
- Bluetooth today is MIDI only (NimBLE, `tr_midi`). ToneRelay's other Bluetooth service (`ble.c`) is a JSON command
  interface from the Raspberry Pi version, not the raw relay.
- The three board problems (issue 6): the C6 co-processor runs factory firmware because the release has no
  `network_adapter.bin` (empty Wi-Fi scan list); after the C6 is updated Bluetooth MIDI stops because
  `esp_hosted_bt_controller_init()/enable()` are never called; the build breaks on the current Rust nightly
  (pin `nightly-2026-07-01`).
- Wi-Fi Direct is not an option: ESP-IDF has no P2P support. The box's own network does the same job.

## Phases

### 0. Our own box (half a day)
- Fork `helrig-box` and `ToneRelay` to `radioactivetoy`; keep `upstream` remotes; the submodule points at our fork.
- Name and look: the code is MIT but the Helrig name, logo and icon are not (see its `NOTICE.md`). Pick our own product
  name, setup page title, mDNS name (`<name>.local`) and replace the logo. The protocol (`/raw`, `_tonerelay._tcp`)
  stays as it is, so Helrig and ToneStrand both keep working.
- Done when: both forks exist, the profile builds under the new name (not yet on a board).

### 1. A build that works (half a day to a day)
- Windows: ESP-IDF v5.5.3 (Espressif installer), Rust nightly pinned to a date that builds (`nightly-2026-07-01`, set in
  `rust-toolchain.toml`), `ldproxy`, `espflash` (we have 4.x), LLVM/libclang, long paths on. The scripts are bash: run
  them from Git Bash.
- If Windows fights us: build in WSL2 or, better, in GitHub Actions on Ubuntu, which also becomes the release pipeline.
- Done when: `scripts/build.sh` produces the same app as the v1.2.0 flasher image (boots, same log), from a clean clone.

### 2. Wi-Fi on the board: the C6 problem (half a day)
- Build the C6 slave image in the pipeline (`build-c6-slave.sh`, target esp32c6) and put it in
  `components/tonerelay/slave_fw/` so `network_adapter.bin` is embedded.
- Setup page: show the C6 version and say plainly when it is old, missing or failed to update, instead of an empty list.
- Done when: a board with factory C6 firmware boots, logs `C6 firmware 2.12.13`, and lists networks.

### 3. Bluetooth on the board: MIDI works again (half a day)
- In `midi_ble.c`: `esp_hosted_bt_controller_init()` then `..._enable()` before `nimble_port_init()` (the patch in issue 6).
- Done when: advertises, pairs from nRF Connect and from an iPad or Mac, program changes reach the pedal, and Wi-Fi
  editing keeps working while connected.
- Send phases 2 and 3 upstream (pull requests to ToneRelay and helrig-box) once they hold on the bench.

### 4. Editing with no Wi-Fi infrastructure: the box's own network (half a day)
- Test: join the box's network, add it in ToneStrand by address (192.168.4.1), pair with BOOT, edit.
- If it works: a "Direct" switch on the setup page that keeps the network up beside the joined one, with a password
  (the default is open today), and a hint in ToneStrand's connect panel.

### 5. Bluetooth editing (3 to 4 days), with Bluetooth MIDI kept
Firmware
- A GATT service `raw relay`: `tx` (notify, box to computer), `rx` (write without response, computer to box), `status`
  (read: hello JSON, protocol, busy). Frames fragmented over the negotiated MTU (up to 512) with a one-byte header and a
  credit scheme so neither side overruns. Same one-client rule as `/raw`: whoever holds the pedal, Wi-Fi or Bluetooth.
- Security: encrypted, bonded link only; a pairing window opened with the BOOT button, like the Wi-Fi token step.
- Keep Bluetooth MIDI: two services on one device, more than one connection (an iPad for MIDI and the computer for
  editing at once). Two switches in setup, stored in NVS: Bluetooth MIDI on/off, Bluetooth editing on/off.
- Enable data length extension and 2M PHY where the C6 supports them.

ToneStrand
- A `BleWire` beside `UsbWire` and `BoxWire` (the `Wire` trait already hides the transport), using `btleplug` on
  Windows. Discovery and pairing in the connect panel; reconnect; a throughput and latency test.
- Done when: a full preset read and write, an IR send and a 128-preset backup work over Bluetooth with the Wi-Fi off;
  measured speed written down. Target: at least 20 kB/s and no dropped session in 30 minutes of editing.

### 6. Improvements (ongoing, in this order)
1. A firmware version string on the box (the protocol notes list it as missing) shown in ToneStrand.
2. Setup page: Wi-Fi, C6, Bluetooth and relay state on one page; the box's address and token shown for a person who needs them.
3. Updates over Wi-Fi from the setup page (the flasher needs a computer and Chrome today).
4. In ToneStrand: say who holds the box when it is busy, offer 192.168.4.1 when no box is found, show the box's firmware version.
5. A watchdog and recovery for the pedal's USB side; docs of what was measured.

## Risks and how we handle them
- **Windows toolchain** for ESP-IDF plus Rust is fussy: fall back to CI builds; developing never depends on the laptop.
- **ESP-Hosted version coupling** (host 2.12.0 with C6 2.12.13): pin both and test the pair together.
- **Windows Bluetooth** (pairing and MTU behaviour): prototype with a small test program before the ToneStrand integration.
- **Hardware needed for every step** from 2 on: the board connected, a COM port, and a pedal. I flash only after you say so.
- **Branding:** the Helrig name and logo are not ours to reuse (phase 0).

## Needed from you
1. The go-ahead to create the two forks on your GitHub account.
2. A product name for the box (or "ToneStrand Box" to start).
3. OK to install ESP-IDF and LLVM here (a few GB), or to do builds in GitHub Actions only.
4. The board connected, and its COM port, for phases 2 onward.

## Status (2026-11-18)

- Phase 0 done: forks `radioactivetoy/tonestrand-box` (branch `tonestrand/own-box`) and `radioactivetoy/ToneRelay` (branch
  `tonestrand/c6-and-ble-fixes`). Names are ToneStrand Box / `tonestrandbox.local` / `_tonestrandbox`; the setup page has no
  Helrig logo. Still to clean: comments and docs that say Helrig, the README, the web flasher.
- Phase 1 done: built in WSL2 Ubuntu (ESP-IDF 5.5.3, nightly-2026-07-01), cold build 60 s. Working copy:
  `~/box/tonestrand-box` in WSL. Image copied to `C:\Users\nesto\Downloads\tonestrand-box-fw` (ELF + partitions.csv).
- Phases 2 and 3 (C6 image embedded, Bluetooth controller started): in the build, **not yet flashed or tested** on the board.
  The earlier C6 image was reused; the script that builds it when missing is untested.
