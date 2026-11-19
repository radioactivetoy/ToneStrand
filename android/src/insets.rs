//! Area clear of the system bars (status bar, navigation bar, display cutout), in egui points.
//!
//! Under edge-to-edge (targetSdk 35) `AndroidApp::content_rect()` is the whole screen, so the bar
//! sizes (applied per edge, only where the content rect reaches the screen edge) come from the decor view's root window insets through JNI. They are cached and read again
//! when the content rect or the native window size changes (rotation, resize), and at least once a
//! second so a missed change heals. A failed read keeps the last good bars.

use eframe::egui::{pos2, Rect};
use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use std::time::{Duration, Instant};
use winit::platform::android::activity::AndroidApp;

type Bars = (i32, i32, i32, i32); // left, top, right, bottom in pixels

pub struct Insets {
    vm: Result<JavaVM, String>,
    last_read: Option<Instant>,
    key: Option<((i32, i32, i32, i32), (i32, i32))>,
    bars: Bars,
    failed: bool,
}

impl Insets {
    pub fn new(app: &AndroidApp) -> Self {
        // SAFETY: vm_as_ptr is the process's JavaVM, valid for the life of the app.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) }.map_err(|e| e.to_string());
        Insets { vm, last_read: None, key: None, bars: (0, 0, 0, 0), failed: false }
    }

    /// The window's content rectangle in points (egui points: pixels / `pixels_per_point`, zoom
    /// included), clear of the system bars; `None` when unknown or empty.
    pub fn content_rect_points(&mut self, app: &AndroidApp, pixels_per_point: f32) -> Option<Rect> {
        let r = app.content_rect();
        let size = app.native_window().map(|w| (w.width(), w.height())).unwrap_or((0, 0));
        let key = ((r.left, r.top, r.right, r.bottom), size);
        // Read again when the geometry changes, and at least once a second so a missed change (a 180
        // degree flip, insets arriving a frame late) heals.
        let stale = self.last_read.map_or(true, |t| t.elapsed() >= Duration::from_secs(1));
        if self.key != Some(key) || stale {
            self.last_read = Some(Instant::now());
            match self.read(app) {
                Ok(Some(b)) => {
                    self.bars = b;
                    self.key = Some(key);
                    // A later, different error is worth logging again.
                    self.failed = false;
                }
                // The decor view is not attached yet: leave the key unset so the next frame reads again.
                Ok(None) => {}
                Err(e) => {
                    // Logged once, not every frame.
                    if !self.failed {
                        log::error!("system bar insets: {e}");
                        self.failed = true;
                    }
                    // Keep the last good bars (zeroing them would flash the UI under the status bar).
                    self.key = Some(key);
                }
            }
        }
        let (l, t, rt, b) = self.bars;
        // Apply a bar's inset only on an edge where the content rect reaches the screen edge: below
        // API 35 the content rect may already exclude the bars, and with the keyboard up (adjustResize)
        // its bottom stops above the keyboard, so subtracting the nav bar again would leave a dead band.
        let (w, h) = size;
        let left = if r.left <= 0 { r.left + l } else { r.left };
        let top = if r.top <= 0 { r.top + t } else { r.top };
        let right = if w > 0 && r.right >= w { r.right - rt } else { r.right };
        let bottom = if h > 0 && r.bottom >= h { r.bottom - b } else { r.bottom };
        if right <= left || bottom <= top || pixels_per_point <= 0.0 {
            return None;
        }
        let p = pixels_per_point;
        Some(Rect::from_min_max(pos2(left as f32 / p, top as f32 / p), pos2(right as f32 / p, bottom as f32 / p)))
    }

    fn read(&self, app: &AndroidApp) -> Result<Option<Bars>, String> {
        let vm = self.vm.as_ref().map_err(|e| format!("no JavaVM: {e}"))?;
        // The android_main thread is already attached by android-activity; this is then a no-op guard.
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let res = env.with_local_frame(16, |env| bars(env, app));
        if res.is_err() && env.exception_check().unwrap_or(false) {
            // A failed call leaves a Java exception pending; clear it or later JNI calls misbehave.
            let _ = env.exception_clear();
        }
        res.map_err(|e| e.to_string())
    }
}

fn bars(env: &mut JNIEnv, app: &AndroidApp) -> Result<Option<Bars>, jni::errors::Error> {
    // SAFETY: activity_as_ptr is the live Activity reference owned by android-activity; JObject does
    // not free it on drop.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
    let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
    let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
    let root = env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
    if root.is_null() {
        return Ok(None);
    }
    let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
    if sdk >= 30 {
        let ty = "android/view/WindowInsets$Type";
        let bars = env.call_static_method(ty, "systemBars", "()I", &[])?.i()?;
        let cutout = env.call_static_method(ty, "displayCutout", "()I", &[])?.i()?;
        let insets = env.call_method(&root, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(bars | cutout)])?.l()?;
        let mut side = |name: &str| -> Result<i32, jni::errors::Error> { env.get_field(&insets, name, "I")?.i() };
        Ok(Some((side("left")?, side("top")?, side("right")?, side("bottom")?)))
    } else {
        // API 26-29: the older accessors (no display cutout).
        let mut side = |name: &str| -> Result<i32, jni::errors::Error> { env.call_method(&root, name, "()I", &[])?.i() };
        Ok(Some((
            side("getSystemWindowInsetLeft")?,
            side("getSystemWindowInsetTop")?,
            side("getSystemWindowInsetRight")?,
            side("getSystemWindowInsetBottom")?,
        )))
    }
}
