# Focus mode: change an effect's character on an X-Y plane

Status: plan (2026-11-23). Nothing built. Branch `focus-mode` (from `main`).

## What it is

Helix Stadium has a "Focus view": for an amp or effect, a square pad whose four corners and centre are five named tones. Dragging
the puck blends the five tones and moves several parameters at once, so you steer the sound by character ("Warm Lead",
"Wash", "Is This On?") instead of by knobs.

ToneStrand gets the same as a **second way to edit a block**, next to the knobs, never instead of them. In Focus mode the knobs
stay on screen and turn as the puck moves.

Sources (read, not built): `C:\Users\nesto\Downloads\StadiumAnalisis` (`AGENT_SPEC_HX_STOMP_FOCUS_VIEW.md`,
`Focus_View_Guide.md`, `focus_view_catalog.json`, a viewer `index.html`), and the Helix Stadium video
https://www.youtube.com/watch?v=ITlq40wzWt0 (not watched by the assistant; the owner knows what it shows).

## What the data says (measured 2026-11-23)

`focus_view_catalog.json` (about 930 KB) has 583 models in 13 categories. Each model has five regions (`origin` at (0,0) and the
corners `x_min_y_min`, `x_min_y_max`, `x_max_y_min`, `x_max_y_max`), each with a name and a value for every parameter, and each
parameter has a behaviour: `interpolate` (blend), `quantize` (switches, sync notes, channels: snap), `exclude` (leave alone).

Against HX Edit 3.82's `Helix.sym` (what the pedal uses):

| Category | Models | Stadium-only (Agoura) | Found on Helix | Parameters found |
|---|---:|---:|---:|---|
| amp | 142 | 31 | 111 | 1380 / 1380 |
| preamp | 112 | 0 | 112 | 993 / 993 |
| distortion | 69 | 0 | 62 | 298 / 299 |
| modulation | 79 | 0 | 69 | 605 / 606 |
| delay | 64 | 0 | 54 | 656 / 663 |
| reverb | 29 | 0 | 22 | 178 / 182 |
| dynamics | 20 | 0 | 19 | 94 / 94 |
| filter | 17 | 0 | 16 | 146 / 146 |
| pitch | 14 | 0 | 9 | 73 / 73 |
| synth | 20 | 0 | 10 | 162 / 162 |
| wah / volume / eq | 11 / 5 / 1 | 0 | 11 / 5 / 1 | all |

About 500 of 583 models map directly (`HD2_` or `L6SPB_` plus the file name, e.g. `DistAlpacaRougeMono`), and almost every
parameter id matches a Helix parameter id. The rest are newer than HX Edit 3.82 (Glitch Delay, Ratchet, Tesselator...): hidden
until the catalog knows them. Cabs and IRs and the standard EQs have their own graphics on Stadium (not this pad): out of scope.

## The maths

Both Stadium documents give the same blend: with `u = |x|`, `v = |y|`, in the quadrant of the puck, weights are
`(1-u)(1-v)` on the centre, `u*v` on the corner, `0.5*u*(1-v)` and `0.5*(1-u)*v` each split between the corner and the
neighbouring corners (the edge midpoints). `interpolate` parameters take the weighted sum; `quantize` take the nearest value held
by any of the five regions (booleans and enums take the heaviest region's value); `exclude` is never touched. At (0,0) and at the
corners the result must equal the stored values exactly.

## Decisions to take before building

1. **Where the data comes from (legal, shapes the design).** The file is Line 6 data, extracted from the owner's Helix Stadium
   Native install. Options: (a) **import**: ToneStrand reads it from the user's own Stadium install (a `--build-focus` tool and an
   Import button; on Android a file pick), nothing Line 6 is shipped; (b) **bundle** a converted file in the repository, the way the
   HX Edit 3.82 catalog is (owner changed that rule on 2026-11-19; this is a different Line 6 product and includes the creative
   region names). Recommendation: write the loader once so both work, ship (a) first, and decide on (b) deliberately. The owner decides.
2. **First categories.** Suggested order: distortion, delay, reverb, modulation (clear tonal meaning, few parameters), then
   amps and preamps (many parameters, Level compensation), then the rest.
3. **Where the toggle lives.** Suggested: a `Knobs | Focus` switch in the block panel header, remembered per model, with a
   setting "open blocks in Focus when available".

## Design

**Core (no UI, in `tonestrand-core`, so Android reuses it)**: `focus` module.
- Types: `FocusModel { symbol, params: Vec<FocusParam { index, id, behaviour }>, regions: [Region; 5] }`, `Region { name, values }`.
- `weights(x, y)`, `evaluate(model, x, y) -> Vec<(index, Value)>`, clamped to the catalog's min/max for each parameter, typed as the
  parameter's value type (float, int, bool).
- Loader from the Stadium JSON, and a compact form keyed by Helix symbol with parameters already resolved to indices (made by the tool).
- Availability: a model has a pad only if it has an entry and the unit supports the model (the catalog's per-unit list).
- Tests: centre and corners equal the stored values; `exclude` never changes; continuity across the axes; every mapped model's
  parameters exist in `Helix.sym`; values inside the catalog's ranges (report violations, do not silently clamp).

**UI (egui, shared by desktop and Android)**: a pad widget with the five names at their positions, a puck, a cross-hair, click or
tap on a corner or the centre to jump, drag to blend. Beside it the normal knob grid, showing the blended values live. Buttons:
Centre, **Revert** (the block as it was when Focus was opened), Corner names as buttons. Touching a knob while in Focus leaves
Focus mode for that block ("custom"), keeps the values, and greys the puck. One undo step per drag. Touch: the pad takes the drag
(no scrolling under it), generous size, no hover-only help.

**Live sending**: a drag changes several parameters at once. Send only what changed since the last tick, newest value wins per
parameter, about 40 to 80 ms per tick (the knobs are paced at 80 ms), and always a final exact send on release. `quantize`
values change when the puck crosses a region: send them on release (or throttled harder) to avoid clicks; decide by listening.
Must be measured through the Box: how many parameter messages per second the pedal takes.

**Files and `.hlx`**: the same evaluator drives the file editors (no sending); parameter names map through the catalog.

**Android**: the pad is egui like everything else, so it comes with the app. Differences to check: drag versus scroll, size on a
10-inch tablet, finger covering the puck (offset the labels), importing the data file through the system picker.

## Phases

0. Decisions above; a short look at how Stadium's own pad behaves (the video) to copy what matters.
1. Core: `focus` module, loader, tests, coverage report against `Helix.sym`.
2. Tool and data: `tonestrand --build-focus <Stadium folder|json> <out>`; the importer; per-model availability.
3. Desktop UI: toggle, pad, live knob mirror, revert, undo; file editors first (no pedal needed), then the live pedal.
4. Live sending and tuning on the HX Stomp through the Box (rate, quantize behaviour, glitches).
5. Review, docs (`docs/FOCUS_MODE.md`), changelog, release (desktop). Then Android: layout and touch checks on the emulator and the tablet
   (the Android tablet checks are paused until the owner resumes them).

## Risks

- The sounds are Stadium's tones on Stomp DSP: close, not identical. The pad should say "from Helix Stadium".
- Parameter units: the guide notes wahs shift ranges; check each category's values against the catalog's ranges.
- Live rate: several messages per drag step may be too many for the Box link; coalescing and a lower tick rate are the answer, to be measured.
- About 80 models are not on HX Edit 3.82 yet: they stay hidden, not broken.
- Legal: see decision 1.
