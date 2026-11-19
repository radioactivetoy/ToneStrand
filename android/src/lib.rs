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
        Box::new(move |cc| {
            Ok(Box::new(AndroidShell {
                inner: App::new(cc, None),
                insets: insets::Insets::new(&app),
                android: app,
                keyboard: keyboard::Keyboard::new(),
                last_rect: None,
                gl_logged: false,
            }))
        }),
    );
    if let Err(e) = run {
        log::error!("run_native: {e}");
    }
    log::info!("window closed; exiting");
    // When the window closes (the activity is destroyed), exit so the next launch starts a fresh
    // process: winit cannot start a second event loop in this one.
    std::process::exit(0);
}

/// The desktop window plus what only Android needs (the system-bar insets).
struct AndroidShell {
    inner: App,
    android: AndroidApp,
    keyboard: keyboard::Keyboard,
    insets: insets::Insets,
    last_rect: Option<eframe::egui::Rect>,
    gl_logged: bool,
}

impl eframe::App for AndroidShell {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        if !self.gl_logged {
            self.gl_logged = true;
            if let Some(gl) = frame.gl() {
                use eframe::glow::HasContext;
                // SAFETY: plain state queries on the current context, on the render thread.
                let (version, srgb_ext, fb_srgb) = unsafe {
                    (
                        gl.get_parameter_string(eframe::glow::VERSION),
                        gl.supported_extensions().contains("GL_EXT_sRGB_write_control"),
                        gl.is_enabled(0x8DB9),
                    )
                };
                log::debug!("GL {version}; EXT_sRGB_write_control {srgb_ext}; FRAMEBUFFER_SRGB enabled {fb_srgb}");
            }
        }
        self.inner.update(ctx, frame);
        let tapped = ctx.input(|i| i.pointer.primary_pressed());
        self.keyboard.after_update(&self.android, ctx.wants_keyboard_input(), tapped);
    }

    fn raw_input_hook(&mut self, ctx: &eframe::egui::Context, raw: &mut eframe::egui::RawInput) {
        // Keep the window's content clear of the system bars; every frame so rotation follows.
        // egui points = pixels / (native scale x UI zoom).
        let native = raw
            .viewports
            .get(&raw.viewport_id)
            .and_then(|v| v.native_pixels_per_point)
            .unwrap_or_else(|| ctx.native_pixels_per_point().unwrap_or(1.0));
        let ppp = native * ctx.zoom_factor();
        if let Some(rect) = self.insets.content_rect_points(&self.android, ppp) {
            if self.last_rect != Some(rect) {
                log::info!("content rect {rect:?} at {ppp} ppp (raw {:?})", self.android.content_rect());
                self.last_rect = Some(rect);
            }
            raw.screen_rect = Some(rect);
        }
        self.keyboard.input_hook(&self.android, raw);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.inner.save(storage);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.inner.on_exit(gl);
    }
}
