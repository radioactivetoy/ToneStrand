//! On-screen keyboard glue: egui never asks Android for the keyboard, and winit drops
//! GameActivity's text, so show it while a text field has focus and hand what is typed to egui.

use tonestrand::touch_text::{events_from_text, KeyPairing, PLACEHOLDER};
use winit::platform::android::activity::input::{ImeOptions, InputType, TextInputAction, TextInputState, TextSpan};
use winit::platform::android::activity::AndroidApp;

pub struct Keyboard {
    shown: bool,
    pairing: KeyPairing,
}

fn reset_text(app: &AndroidApp) {
    // Selection is in characters: the placeholder is one.
    app.set_text_input_state(TextInputState {
        text: PLACEHOLDER.to_owned(),
        selection: TextSpan { start: 1, end: 1 },
        compose_region: None,
    });
}

impl Keyboard {
    pub fn new() -> Self {
        Self { shown: false, pairing: KeyPairing::new() }
    }

    /// Call after each frame: show or hide the keyboard when a text field gains or loses focus.
    /// `tapped`: a primary pointer press happened this frame; tapping a still-focused field after
    /// Back dismissed the keyboard brings it back.
    pub fn after_update(&mut self, app: &AndroidApp, wants_keyboard: bool, tapped: bool) {
        if wants_keyboard == self.shown {
            if wants_keyboard && tapped {
                app.show_soft_input(true);
            }
            return;
        }
        self.pairing.clear();
        log::info!("on-screen keyboard: {}", if wants_keyboard { "show" } else { "hide" });
        if wants_keyboard {
            // Plain text with no suggestions, so letters arrive as they are typed.
            app.set_ime_editor_info(
                InputType::TYPE_CLASS_TEXT
                    | InputType::TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                    | InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS
                    | InputType::TYPE_TEXT_FLAG_MULTI_LINE,
                TextInputAction::Done,
                ImeOptions::IME_FLAG_NO_FULLSCREEN | ImeOptions::IME_FLAG_NO_ENTER_ACTION,
            );
            reset_text(app);
            app.show_soft_input(true);
        } else {
            app.hide_soft_input(true);
        }
        self.shown = wants_keyboard;
    }

    /// Call before each frame: pair key events and turn typed text into egui events.
    pub fn input_hook(&mut self, app: &AndroidApp, raw: &mut eframe::egui::RawInput) {
        let seen = self.pairing.fix(&mut raw.events);
        if !self.shown {
            return;
        }
        let state = app.text_input_state();
        if state.compose_region.is_some() {
            return;
        }
        if let Some(events) = events_from_text(&state.text, seen) {
            raw.events.extend(events);
            reset_text(app);
        }
    }
}
