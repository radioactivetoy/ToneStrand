//! Turning what Android's on-screen keyboard reports into egui events. Pure: no Android here.

use eframe::egui::{Event, Key, Modifiers};

/// The text Android's keyboard edits is kept as this one character, cursor after it: new letters
/// show up next to it, a backspace removes it.
pub const PLACEHOLDER: &str = "\u{200B}";

fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
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
    /// not seen. A key really held (hardware keyboard) is left alone. Returns whether any
    /// Backspace key event was present.
    pub fn fix(&mut self, events: &mut Vec<Event>) -> bool {
        let mut saw_backspace = false;
        let mut i = 0;
        while i < events.len() {
            if let Event::Key { key, pressed, modifiers, .. } = events[i].clone() {
                saw_backspace |= key == Key::Backspace;
                if pressed {
                    self.held.push(key);
                } else if let Some(at) = self.held.iter().position(|k| *k == key) {
                    self.held.remove(at);
                } else {
                    events.insert(i, key_event(key, true, modifiers));
                    i += 1;
                }
            }
            i += 1;
        }
        saw_backspace
    }
}

/// Events for the keyboard's text state, or None when nothing was typed. The caller resets the
/// text state after Some.
pub fn events_from_text(text: &str, saw_backspace_key: bool) -> Option<Vec<Event>> {
    if text == PLACEHOLDER {
        return None;
    }
    let mut out = Vec::new();
    if !text.contains(PLACEHOLDER) && !saw_backspace_key {
        out.push(key_event(Key::Backspace, true, Modifiers::NONE));
        out.push(key_event(Key::Backspace, false, Modifiers::NONE));
    }
    let typed = text.replace(PLACEHOLDER, "");
    for line in typed.split_inclusive('\n') {
        let t = line.trim_end_matches('\n');
        if !t.is_empty() {
            out.push(Event::Text(t.to_owned()));
        }
        if line.ends_with('\n') {
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

    #[test]
    fn letter_typed_before_placeholder() {
        assert_eq!(events_from_text("w\u{200B}", false), Some(vec![Event::Text("w".into())]));
    }
    #[test]
    fn two_letters_in_one_frame() {
        assert_eq!(events_from_text("\u{200B}ab", false), Some(vec![Event::Text("ab".into())]));
    }
    #[test]
    fn nothing_typed() {
        assert_eq!(events_from_text(PLACEHOLDER, false), None);
    }
    #[test]
    fn backspace_seen_once_when_key_and_text_both_report_it() {
        assert_eq!(events_from_text("", true), Some(vec![]));
    }
    #[test]
    fn backspace_from_text_alone() {
        assert_eq!(events_from_text("", false), Some(vec![key(Key::Backspace, true), key(Key::Backspace, false)]));
    }
    #[test]
    fn enter_from_newline() {
        assert_eq!(
            events_from_text("\u{200B}ok\n", false),
            Some(vec![Event::Text("ok".into()), key(Key::Enter, true), key(Key::Enter, false)])
        );
    }
    #[test]
    fn lone_release_gets_a_press() {
        let mut p = KeyPairing::new();
        let mut ev = vec![key(Key::Backspace, false)];
        assert!(p.fix(&mut ev));
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
}
