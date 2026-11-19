//! The ToneStrand app on Android: the desktop library's window, started inside a GameActivity.

mod insets;
mod keyboard;

use tonestrand::window::App;
use winit::platform::android::activity::AndroidApp;

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("tonestrand"));
    // Settings, library and backups go to the app's own storage.
    if let Some(dir) = app.internal_data_path() {
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("XDG_DOCUMENTS_DIR", dir.join("Documents"));
        std::env::set_var("TONESTRAND_LIBRARY", dir.join("Library"));
    }
    std::panic::set_hook(Box::new(|p| log::error!("panic: {p}")));
    let options = eframe::NativeOptions { android_app: Some(app.clone()), ..Default::default() };
    let run = eframe::run_native(
        "ToneStrand",
        options,
        Box::new(move |cc| Ok(Box::new(AndroidShell { inner: App::new(cc, None), android: app }))),
    );
    if let Err(e) = run {
        log::error!("run_native: {e}");
    }
    // Back destroys the activity and ends the window; winit cannot start a second event loop in
    // this process, so end it and let the next launch start a fresh one.
    std::process::exit(0);
}

/// The desktop window plus what only Android needs (keyboard, insets: added later).
struct AndroidShell {
    inner: App,
    #[allow(dead_code)]
    android: AndroidApp,
}

impl eframe::App for AndroidShell {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        self.inner.update(ctx, frame);
    }

    fn raw_input_hook(&mut self, _ctx: &eframe::egui::Context, _raw: &mut eframe::egui::RawInput) {}

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.inner.save(storage);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.inner.on_exit(gl);
    }
}
