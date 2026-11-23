# Focus mode

Focus mode lets you change an effect's **character** instead of turning its knobs one at a time. The effect's photo becomes a
pad. Its middle and its four corners are five named tones (an amp might have "Wall" in the middle and "Miles", "Spirit", "Noon" and
"Rumored" in the corners). Where you put the puck decides how much of each tone you get, and several parameters move together.
The knobs beside the pad turn with the puck, and on the loaded preset the pedal follows live.

## Where the tones come from

The tones, their names and their values are Line 6's, taken from **Helix Stadium Native** (its `focus-view-xy-defaults` folder, version
1.4.1) and matched to the effects of the Helix-family pedals. ToneStrand is not affiliated with Line 6; see "Third-party material and
trademarks" in the [README](../README.md). They are Stadium's tones played on the pedal's own DSP, so the sound is close to Stadium's
Focus view but not identical. A parameter that holds the same value in all five tones is not part of Focus and keeps the value you
gave it (about a thousand parameters across the effects, for example a Hiwatt's Hum, Ripple and Bias X).

## Using it

1. Select an amp or effect in the chain. If it has Focus tones, a **Focus** button sits in the block's header, just before
   "Change effect...". Press it (press it again to go back to the knobs). ToneStrand remembers the choice for that effect.
2. **Drag** on the pad, or **tap** it. The puck follows; the parameters are blended between the tones, and the knobs move with it. A
   tap within a short distance of a ring (the middle or a corner) lands exactly on that tone.
3. The line under the pad names the tone that counts most at the puck, with its share. "Centre: ..." names the middle tone.
4. **Centre** puts the puck in the middle. **Revert** goes back to how the effect was before your first move in this session.
5. **Undo** and **Redo** appear on the pad for the preset loaded on the pedal (it has no undo of its own). On a preset file or a
   preset read from a slot, one drag, tap, Centre or Revert is one step of the normal undo (Ctrl+Z / Y, the undo buttons).

### What the pad shows when you have not touched it

Nothing is written, sent or marked as changed until you click or drag on the pad or press one of its buttons. Turning Focus on,
switching to another block or reloading the preset changes nothing. Instead the pad looks at the values the effect has now:

- **A ring** (outline, no fill) where those values sit on the pad, with "On the pad: <tone> NN%": the effect is at, or very near, a
  point of the pad (for example you saved the preset from the pad). A tap on the pad always starts from the point you tap.
- **No puck** and "Set by hand: closest tone <name>": the values do not match any point (you set knobs yourself). If you turn a knob
  after a move, the solid puck becomes a ring or disappears by the same rule.

### The knobs beside the pad

The knobs Focus moves come first. Under "Not moved by Focus" the other knobs follow, dimmed but fully editable. A small dot before a
knob means its value has changed since before your first move; after Revert the dots are gone.

## Which effects have it

About 595 effect versions (a Mono and a Stereo version are counted apart): amps, preamps, distortions, modulation, delays, reverbs,
dynamics, filters, wahs, synths, pitch and volume effects. Cabs, IRs, the plain EQs, the Helix Stadium-only amps and the effects newer
than HX Edit 3.82's list have no Focus; neither do effects whose parameters are all the same in the five tones.

## Settings

Settings, **Behaviour**: "Open effects in Focus mode when available" opens every effect that has Focus that way. The Focus button
still switches one effect, and that effect keeps your choice.

## Touch

On a touch screen (the Android app, still unfinished) the pad is larger, its rings and buttons are bigger, the puck has a wide halo so it
shows around a fingertip, and names fade farther from the puck. The pad is as tall as the window allows and the panel scrolls.

## Limits

- On the loaded preset the pad sends its changes in small batches (about every 50 ms, newest value only, tiny moves skipped), so a very
  fast drag can lag behind the pointer a little; the final value is always sent when you let go.
- Switches and choices that snap (on/off, note sync...) change on the pedal when you release the pad, not during the drag.
- Focus is in the editor for pedal presets (live or in a file), not in the `.hlx` file editor yet.
- Checked on an HX Stomp. The other units share the effects but have not been tried; the Android app has not been tried on a tablet.

## For developers

- The code is in `core/src/focus.rs` (tones, blend, `locate`, resolver, pack) and `src/editors/focus_pad.rs` (the pad). The design
  and the phases are in [docs/superpowers/specs/2026-11-23-focus-mode-design.md](superpowers/specs/2026-11-23-focus-mode-design.md).
- The pack is made from Helix Stadium Native's folder with
  `tonestrand --export-focus "<Stadium Native>\res\meta-data\focus-view-xy-defaults" focus <version>`; it writes
  `focus/<version>/focus.json.gz` and `meta.json` and refuses if that folder exists. `core/build.rs` embeds the newest version. See
  [focus/README.md](../focus/README.md).
- For pictures (never read from your settings): `TONESTRAND_FOCUS=1` shows Focus by default, `TONESTRAND_FOCUS_AT=x,y` puts the puck
  there as a gesture, `TONESTRAND_FOCUS_GHOST=x,y` writes the values at that point with no gesture (the ring shows), and
  `TONESTRAND_TOUCH=1` uses the touch layouts. See [BUILDING.md](BUILDING.md).
