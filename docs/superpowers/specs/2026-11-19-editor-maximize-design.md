# Editor: maximize the chain or the block panel

## Why

On the owner's Lenovo Tab M10 FHD Plus (1920x1200 px at 240 dpi = 1280x800 egui points) the
editor's chain canvas plus the preset bars use nearly all the height, and the block panel
(the knobs) is squeezed to its header row. Live editing, the top feature, needs the knobs.
Small laptop screens on Windows have the same problem in a milder form.

## What (agreed with the owner)

The editor page (`App::studio_central`, `src/window/mod.rs`) has three layouts:

- **Normal** (default): exactly today's layout. Nothing changes for anyone who never clicks.
- **Block maximized**: the chain canvas collapses to a one-row **strip of chips**, one per
  block in the chain (its category colour and short name), in chain order. Tapping a chip
  selects that block, exactly as clicking it in the chain does. The block / controllers panel
  gets the rest of the height.
- **Chain maximized**: the block / controllers panel collapses to its header row (the
  Block / Controllers switch and the selected block's name); the chain canvas gets the rest.

Each of the two areas has a small maximize / restore button at the right of its header. A
maximized area's button restores Normal. The choice is remembered in settings across runs.
It applies to Windows and Android alike (same code).

## Not in this change

- The chain strip is for choosing a block only: no drag, no context menu, no add.
- Folding the left preset list away; a compact phone layout (later work).
- "Getting started" already stays hidden once dismissed (`settings.hide_getting_started`).

## Approaches considered

1. Auto zoom-out on small screens: cheap but makes everything smaller; rejected by the owner
   in favour of this.
2. Maximize toggle (picked by the owner), with the chip strip so blocks stay reachable while
   the knobs are maximized (my addition, agreed).
3. Full compact layout: too big for now; comes with the phone layout.
