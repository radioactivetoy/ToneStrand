//! Pacing of live knob sends: a knob is heard while it turns (a send every `LIVE_STEP` at most), and the
//! final value always goes on release.

use std::time::{Duration, Instant};

use eframe::egui;

/// The least time between two sends while a knob is being dragged.
pub const LIVE_STEP: Duration = Duration::from_millis(80);

/// Whether to send a knob's value to the pedal now: on release when anything changed since the last send,
/// or while turning when it changed and no send is recent.
pub fn should_send(last_sent: Option<Instant>, now: Instant, released: bool, changed: bool) -> bool {
    if !changed {
        return false;
    }
    released || last_sent.is_none_or(|t| now.saturating_duration_since(t) >= LIVE_STEP)
}

/// Whether an unsent change is still the knob's own: the knob was drawn on the previous pass (or this one).
/// A knob that was not drawn since (its block deselected, another preset shown, scrolled away) drops it.
pub fn pending_still_valid(last_pass: u64, this_pass: u64) -> bool {
    this_pass.saturating_sub(last_pass) <= 1
}

/// The per-knob decision at a live send site. `moved` is the knob's new value this frame, `commit` its
/// release (or a wheel, typed or keyboard edit). Keeps the last send time and an unsent change in egui
/// temp memory under `key`. A release with no unsent change still sends, as a commit always did.
pub fn live_send(ui: &egui::Ui, key: impl std::hash::Hash, moved: bool, commit: bool) -> bool {
    let id = egui::Id::new(("live-send", key));
    let pass = ui.ctx().cumulative_pass_nr();
    let (last, pending, seen): (Option<Instant>, bool, u64) = ui.data(|d| d.get_temp(id)).unwrap_or((None, false, pass));
    let pending = (pending && pending_still_valid(seen, pass)) || moved;
    if !pending && !commit {
        ui.data_mut(|d| d.insert_temp(id, (last, false, pass)));
        return false;
    }
    let now = Instant::now();
    let send = should_send(last, now, commit, pending || commit);
    if send {
        ui.data_mut(|d| d.insert_temp(id, (if commit { None } else { Some(now) }, false, pass)));
    } else {
        ui.data_mut(|d| d.insert_temp(id, (last, pending, pass)));
        // Wake up in time to send the held-back value even if the pointer stops moving.
        ui.ctx().request_repaint_after(LIVE_STEP);
    }
    send
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_first_change_goes_at_once() {
        let t = Instant::now();
        assert!(should_send(None, t, false, true));
    }
    #[test]
    fn pacing_waits_between_sends() {
        let t = Instant::now();
        assert!(!should_send(Some(t), t + Duration::from_millis(40), false, true));
        assert!(should_send(Some(t), t + LIVE_STEP, false, true));
    }
    #[test]
    fn pacing_release_always_sends_the_last_value() {
        let t = Instant::now();
        assert!(should_send(Some(t), t + Duration::from_millis(5), true, true));
    }
    #[test]
    fn pacing_unsent_change_expires_when_the_knob_was_not_drawn() {
        assert!(pending_still_valid(7, 8));
        assert!(pending_still_valid(8, 8));
        assert!(!pending_still_valid(7, 9));
        assert!(!pending_still_valid(0, 500));
    }
    #[test]
    fn pacing_nothing_changed_nothing_sent() {
        let t = Instant::now();
        assert!(!should_send(Some(t), t + LIVE_STEP, false, false));
    }
}
