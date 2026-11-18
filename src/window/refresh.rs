//! Which copies of the loaded preset to show. Each edit that the pedal answers with a fresh copy adds one to a count;
//! when several are sent in a row only the last copy is current, and an older one still on its way would put the state
//! from before the newest edit back on screen.

use std::time::{Duration, Instant};

/// How long a copy is waited for before the count is given up (an edit whose answer never came).
const PATIENCE: Duration = Duration::from_secs(8);

#[derive(Default)]
pub struct RefreshGuard {
    due: u32,
    last_sent: Option<Instant>,
}

impl RefreshGuard {
    /// An edit that gets a copy back was sent.
    pub fn sent(&mut self, now: Instant) {
        self.due += 1;
        self.last_sent = Some(now);
    }

    /// An edit failed: no copy comes for it.
    pub fn failed(&mut self) {
        self.due = self.due.saturating_sub(1);
    }

    /// A copy arrived: whether it is the current one and should be shown.
    pub fn arrived(&mut self, now: Instant) -> bool {
        if self.due == 0 {
            return true;
        }
        if self.last_sent.is_some_and(|t| now.duration_since(t) > PATIENCE) {
            self.due = 0;
            return true;
        }
        self.due -= 1;
        self.due == 0
    }

    /// Start again (a new connection).
    pub fn reset(&mut self) {
        *self = RefreshGuard::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_of_several_copies_is_shown() {
        let t = Instant::now();
        let mut g = RefreshGuard::default();
        assert!(g.arrived(t), "a copy nobody asked for (the pedal switched preset) is shown");
        g.sent(t);
        g.sent(t);
        g.sent(t);
        assert!(!g.arrived(t));
        assert!(!g.arrived(t));
        assert!(g.arrived(t), "the third is current");
        assert!(g.arrived(t), "and the count is back to nothing");
    }

    #[test]
    fn a_failed_edit_does_not_hold_back_the_next_copy() {
        let t = Instant::now();
        let mut g = RefreshGuard::default();
        g.sent(t);
        g.sent(t);
        g.failed();
        assert!(g.arrived(t));
        g.failed();
        assert!(g.arrived(t), "failures with nothing due are harmless");
    }

    #[test]
    fn a_copy_that_never_comes_is_given_up() {
        let t = Instant::now();
        let mut g = RefreshGuard::default();
        g.sent(t);
        g.sent(t);
        assert!(g.arrived(t + Duration::from_secs(9)), "waited long enough: show what came");
        assert!(g.arrived(t + Duration::from_secs(9)));
        g.sent(t);
        g.reset();
        assert!(g.arrived(t));
    }
}
