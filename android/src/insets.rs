//! Area clear of the system bars (status bar, navigation bar), in egui points.
//!
//! Under edge-to-edge (targetSdk 35) `AndroidApp::content_rect()` is the whole screen, so the bar
//! sizes come from the decor view's root window insets through JNI.

use eframe::egui::{pos2, Rect};
use jni::objects::{JObject, JValue};
use winit::platform::android::activity::AndroidApp;

/// System bar sizes in pixels: (left, top, right, bottom).
fn system_bars(app: &AndroidApp) -> Result<(i32, i32, i32, i32), jni::errors::Error> {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _)? };
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
    let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
    let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
    let root = env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
    if root.is_null() {
        return Ok((0, 0, 0, 0));
    }
    let kind = env.call_static_method("android/view/WindowInsets$Type", "systemBars", "()I", &[])?.i()?;
    let insets = env.call_method(&root, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(kind)])?.l()?;
    let mut side = |name: &str| -> Result<i32, jni::errors::Error> { env.get_field(&insets, name, "I")?.i() };
    Ok((side("left")?, side("top")?, side("right")?, side("bottom")?))
}

/// The window's content rectangle in points, clear of the system bars; `None` when unknown or empty.
pub fn content_rect_points(app: &AndroidApp, pixels_per_point: f32) -> Option<Rect> {
    let r = app.content_rect();
    let (l, t, rt, b) = match system_bars(app) {
        Ok(v) => v,
        Err(e) => {
            log::error!("system bar insets: {e}");
            (0, 0, 0, 0)
        }
    };
    let (left, top, right, bottom) = (r.left + l, r.top + t, r.right - rt, r.bottom - b);
    if right <= left || bottom <= top || pixels_per_point <= 0.0 {
        return None;
    }
    let s = 1.0 / pixels_per_point;
    Some(Rect::from_min_max(pos2(left as f32 * s, top as f32 * s), pos2(right as f32 * s, bottom as f32 * s)))
}
