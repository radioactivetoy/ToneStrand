# Test data

Used by `nam::tests::matches_nam2ir_py_byte_for_byte`.

- `MES DRECF RD Gain2 BAL O808 CAB.nam`: a real NAM A2 Lite capture (input level 15 dBu,
  loudness -13 dB).
- `reference-nam2ir.wav`, `reference-nam2ir-no-level-no-loudness.wav`: what the
  hx-stomp-nam-lite project's own `nam2ir.py` writes for that capture, with default options and
  with `--no-level --no-loudness`.
- `LT TV MIX 7 500N.wav`: an ordinary cab IR, for trying IR uploads by hand.

Check you may publish the capture and the IR before making the repository public.
