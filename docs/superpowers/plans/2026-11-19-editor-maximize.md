# Editor maximize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the editor page maximize either the chain canvas or the block panel, with a chip strip that keeps blocks selectable while the block panel is maximized.

**Architecture:** A three-state layout (`Normal`, `ChainMax`, `BlockMax`) stored in `Settings` drives the panel heights in `App::studio_central`. A new editor "part" 4 draws a one-row chip strip from both editors (`device_editor` and `hlx_editor`), reusing their own block lists, names, colours and `selected` index.

**Tech Stack:** Rust, egui/eframe 0.31, serde.

**Spec:** docs/superpowers/specs/2026-11-19-editor-maximize-design.md

## Global Constraints

- Normal layout must look exactly as before (same heights, same panels); the only visible addition is one small button in each area's header.
- Same code on Windows and Android; no `cfg(target_os)` in this change.
- `Settings` lives in `core/src/boxes.rs` (`#[serde(default)]`): a new field needs only a default; old settings files must still load.
- `cargo fmt`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` stay clean.
- The `android/` crate must still build: `cargo check --manifest-path android/Cargo.toml --target aarch64-linux-android` is NOT required (needs NDK); keep the change inside the desktop library's existing APIs.
- Touch: the new buttons and chips must be at least 32 points tall (touch targets); use `theme::touch()` if a size helper is needed.
- Docs/text: plain English like the rest of the UI; hover text on the buttons ("Give the knobs the whole page", "Give the chain the whole page", "Back to the normal layout").

## Review Focus

- A document with no blocks (empty preset): the strip shows a muted "No blocks" label, no panic.
- Switching tabs between an .hlx document and a pedal document while maximized: the strip shows the right document's blocks and the selection works in both.
- Controllers tab while Block maximized: the strip still selects blocks and the panel shows controllers (selection is kept for when Block is picked).
- A very long chain (Helix: up to 32 blocks over 2 paths): the strip scrolls horizontally instead of growing the window.
- Old settings file without the new field loads as Normal.

---

### Task 1: The chip strip (editor part 4)

**Files:**
- Modify: `src/editors/mod.rs` (part helpers near `set_part`, line ~1036; new `strip_part()` and a shared `chip_strip` widget)
- Modify: `src/editors/device.rs` (`device_editor`)
- Modify: `src/editors/hlx_file.rs` (`hlx_editor`)
- Test: unit tests in `src/editors/mod.rs` (`#[cfg(test)]` module, or the existing one there)

**Interfaces:**
- Produces: `editors::set_part(4)` makes `device_editor` / `hlx_editor` draw only the chip strip (no chain, no block panel). `chain_part()` and `panel_part()` must both return false for part 4.
- Produces: `pub struct Chip { pub sel: usize, pub name: String, pub colour: Color32 }` and `pub fn chip_strip(ui: &mut egui::Ui, chips: &[Chip], selected: &mut Option<usize>) -> bool` (true when a chip was tapped) in `src/editors/mod.rs`.

- [ ] **Step 1: Write failing tests** for the part flags: after `set_part(4)`, `chain_part()` is false, `panel_part()` is false, `strip_part()` is true; after `set_part(1)` / `set_part(2)` / `set_part(0)` the old behaviour holds (1: chain only, 2: panel only, 0: both) and `strip_part()` is false. Reset with `set_part(0)` at the end (the flag is a global atomic; keep these in one test function so they don't race).
- [ ] **Step 2: Run** `cargo test -p tonestrand part_flags` — expect FAIL (no `strip_part`).
- [ ] **Step 3: Implement** `strip_part()` (`p == 4`); `chain_part()` becomes `p != 2 && p != 4`, `panel_part()` becomes `p != 1 && p != 4` (parts 0–3 behave exactly as now).
- [ ] **Step 4: Implement `chip_strip`**: a horizontal `ScrollArea` (id salt "chip-strip", no vertical growth) of chips; each chip is a rounded rect filled with the block's colour (selected one with an `accent()` 2 pt outline), short name in a contrasting ink, min height 32 pt, tap sets `*selected = Some(chip.sel)`. Empty `chips`: muted label "No blocks".
- [ ] **Step 5: Wire part 4 in `device_editor`**: when `strip_part()`, build chips from `preset.slots()` — every slot with a model (not Input/Output/Split/Join/Empty), in path then position order, `sel = slot.global`, name and colour as the chain uses for that slot (reuse the existing describe/colour helpers there) — call `chip_strip`, return.
- [ ] **Step 6: Wire part 4 in `hlx_editor`**: after `blocks` is built (riding cabs already filtered, line ~154) and the default selection is set, when `strip_part()` build chips from `blocks` in order with `sel = index into blocks` (the same index `selected` already uses) and `name_of(&b.model)` for name and colour; call `chip_strip`; return before drawing the chain.
- [ ] **Step 7: Run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt` — all clean.
- [ ] **Step 8: Commit** `Editor: chip strip of the chain's blocks (editor part 4)`.

### Task 2: The three layouts and their buttons

**Files:**
- Modify: `core/src/boxes.rs` (`Settings`: new field)
- Modify: `src/window/mod.rs` (`studio_central`, line ~1346)
- Modify: `docs/BUILDING.md` (document `TONESTRAND_LAYOUT` next to the other `TONESTRAND_*` variables)
- Test: `core` unit test for the settings default

**Interfaces:**
- Consumes: Task 1's `editors::set_part(4)` via `self.editor(ui, i, 4)`.
- Produces: `Settings::editor_layout: u8` — 0 Normal, 1 chain maximized, 2 block maximized. Default 0.

- [ ] **Step 1: Write a failing test** in core: `serde_json::from_str::<Settings>("{}")` gives `editor_layout == 0`; a value 2 round-trips through serialize/deserialize.
- [ ] **Step 2: Run** it — FAIL; **add the field** (doc comment: "Editor page layout: 0 normal, 1 chain maximized, 2 block panel maximized."); run — PASS.
- [ ] **Step 3: `studio_central`**: read `layout = self.settings.editor_layout` (values above 2 treated as 0), overridden at start-up by env `TONESTRAND_LAYOUT=chain|block` (for screenshots, read like `TONESTRAND_DOCK_TAB`; not saved).
  - Normal: today's code unchanged, plus a maximize button at the right of the chain header row (beside the help "?") and one at the right of the Block/Controllers header row.
  - Block maximized: the "studio-canvas" panel shows only a header row (with the restore button) and `self.editor(ui, i, 4)`; its height is the strip's height (about 32 pt chips + margins; use `exact_height` of header + 48 pt). The central panel is unchanged (it takes the rest).
  - Chain maximized: the "studio-canvas" panel takes all height except the dock header (no 60% cap; `ui.available_height()` minus about 48 pt); the dock shows only its header row (Block/Controllers switch, help, restore button) and no editor body.
  - Buttons: small icon-like buttons, at least 32 pt tall; label "⛶" is not in egui's default font — use text "Maximize" / "Restore" with `small_button`-like look, or draw two simple rect glyphs; hover texts from Global Constraints. Clicking sets `self.settings.editor_layout` and calls `self.settings.save()`.
- [ ] **Step 4: Run** `cargo test --workspace`, clippy, fmt — clean.
- [ ] **Step 5: Screenshots** at the owner's tablet size: `TONESTRAND_SIZE=1280x800` (check `src/main.rs:13` for the exact format) with `cargo run -- --screenshot target/shot-normal.png`, then with `TONESTRAND_LAYOUT=block` and `TONESTRAND_LAYOUT=chain`. Also one Normal at the default size to compare with main (`git stash` not needed: build main in a worktree only if a difference is suspected). Check: Normal unchanged apart from the two buttons; block-max shows the strip and the knobs; chain-max shows the chain and the dock header.
- [ ] **Step 6: Commit** `Editor: maximize the chain or the block panel (remembered in settings)`.
