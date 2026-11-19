//! Turning what Android's on-screen keyboard reports into egui events. Pure: no Android here.

use eframe::egui::{Event, Key, Modifiers};

/// The text Android's keyboard edits is kept as this one character, cursor after it: new letters
/// show up next to it, a backspace removes it.
pub const PLACEHOLDER: &str = "\u{200B}";

fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

/// Which keys had events in a frame (the text state may report the same keypress too).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Seen {
    pub backspace: bool,
    pub enter: bool,
}

/// Keeps key presses and releases paired.
#[derive(Default)]
pub struct KeyPairing {
    held: Vec<Key>,
}

impl KeyPairing {
    pub fn new() -> Self {
        Self::default()
    }

    /// The on-screen keyboard sends releases only: put a press before each release whose press was
    /// not seen. A key really held (hardware keyboard) is left alone. Returns which of
    /// Backspace and Enter had key events.
    pub fn fix(&mut self, events: &mut Vec<Event>) -> Seen {
        let mut seen = Seen::default();
        let mut i = 0;
        while i < events.len() {
            if let Event::Key { key, pressed, modifiers, .. } = events[i].clone() {
                seen.backspace |= key == Key::Backspace;
                seen.enter |= key == Key::Enter;
                if pressed {
                    // Autorepeat presses come with one release: hold the key once.
                    if !self.held.contains(&key) {
                        self.held.push(key);
                    }
                } else if self.held.contains(&key) {
                    self.held.retain(|k| *k != key);
                } else {
                    events.insert(i, key_event(key, true, modifiers));
                    i += 1;
                }
            }
            i += 1;
        }
        seen
    }

    /// Forget keys thought held (a release may have been lost).
    pub fn clear(&mut self) {
        self.held.clear();
    }
}

/// Events for the keyboard's text state, or None when nothing was typed. The caller resets the
/// text state after Some.
pub fn events_from_text(text: &str, seen: Seen) -> Option<Vec<Event>> {
    if text == PLACEHOLDER {
        return None;
    }
    let mut out = Vec::new();
    if !text.contains(PLACEHOLDER) && !seen.backspace {
        out.push(key_event(Key::Backspace, true, Modifiers::NONE));
        out.push(key_event(Key::Backspace, false, Modifiers::NONE));
    }
    let typed = text.replace(PLACEHOLDER, "");
    for line in typed.split_inclusive('\n') {
        let t = line.trim_end_matches('\n');
        if !t.is_empty() {
            out.push(Event::Text(t.to_owned()));
        }
        if line.ends_with('\n') && !seen.enter {
            out.push(key_event(Key::Enter, true, Modifiers::NONE));
            out.push(key_event(Key::Enter, false, Modifiers::NONE));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Key, Modifiers};

    fn key(k: Key, pressed: bool) -> Event {
        Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
    }

    const BS: Seen = Seen { backspace: true, enter: false };

    #[test]
    fn letter_typed_before_placeholder() {
        assert_eq!(events_from_text("w\u{200B}", Seen::default()), Some(vec![Event::Text("w".into())]));
    }
    #[test]
    fn two_letters_in_one_frame() {
        assert_eq!(events_from_text("\u{200B}ab", Seen::default()), Some(vec![Event::Text("ab".into())]));
    }
    #[test]
    fn nothing_typed() {
        assert_eq!(events_from_text(PLACEHOLDER, Seen::default()), None);
    }
    #[test]
    fn backspace_seen_once_when_key_and_text_both_report_it() {
        assert_eq!(events_from_text("", BS), Some(vec![]));
    }
    #[test]
    fn backspace_from_text_alone() {
        assert_eq!(events_from_text("", Seen::default()), Some(vec![key(Key::Backspace, true), key(Key::Backspace, false)]));
    }
    #[test]
    fn enter_from_newline() {
        assert_eq!(
            events_from_text("\u{200B}ok\n", Seen::default()),
            Some(vec![Event::Text("ok".into()), key(Key::Enter, true), key(Key::Enter, false)])
        );
    }
    #[test]
    fn lone_release_gets_a_press() {
        let mut p = KeyPairing::new();
        let mut ev = vec![key(Key::Backspace, false)];
        assert!(p.fix(&mut ev).backspace);
        assert_eq!(ev, vec![key(Key::Backspace, true), key(Key::Backspace, false)]);
    }
    #[test]
    fn held_hardware_key_not_doubled() {
        let mut p = KeyPairing::new();
        let mut a = vec![key(Key::Backspace, true)];
        p.fix(&mut a);
        let mut b = vec![key(Key::Backspace, false)];
        p.fix(&mut b);
        assert_eq!(b, vec![key(Key::Backspace, false)]);
    }

    #[test]
    fn autorepeat_then_lone_release_still_paired() {
        let mut p = KeyPairing::new();
        let mut a = vec![key(Key::Backspace, true), key(Key::Backspace, true)];
        p.fix(&mut a);
        let mut b = vec![key(Key::Backspace, true)];
        p.fix(&mut b);
        let mut c = vec![key(Key::Backspace, false)];
        p.fix(&mut c);
        assert_eq!(c, vec![key(Key::Backspace, false)]);
        let mut d = vec![key(Key::Backspace, false)];
        p.fix(&mut d);
        assert_eq!(d, vec![key(Key::Backspace, true), key(Key::Backspace, false)]);
    }
    #[test]
    fn enter_key_and_newline_count_once() {
        let seen = Seen { backspace: false, enter: true };
        assert_eq!(events_from_text("\u{200B}\n", seen), Some(vec![]));
    }
}
