//! Wi-Fi multicast lock. Android drops multicast packets to save battery, which hides the Box's
//! mDNS announcements; holding a `WifiManager.MulticastLock` lets them through. Taken once at
//! start and held for the life of the process (the lock is not reference counted).

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use winit::platform::android::activity::AndroidApp;

/// Acquire the lock; logs the outcome and never fails start-up.
pub fn acquire(app: &AndroidApp) {
    match try_acquire(app) {
        Ok(()) => log::info!("multicast lock acquired"),
        Err(e) => log::error!("multicast lock not acquired: {e}"),
    }
}

fn try_acquire(app: &AndroidApp) -> Result<(), String> {
    // SAFETY: vm_as_ptr is the process's JavaVM, valid for the life of the app.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) }.map_err(|e| e.to_string())?;
    // The android_main thread is already attached by android-activity; this is then a no-op guard.
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let res = env.with_local_frame(16, |env| lock(env, app));
    if res.is_err() && env.exception_check().unwrap_or(false) {
        // A failed call leaves a Java exception pending; clear it or later JNI calls misbehave.
        let _ = env.exception_clear();
    }
    res.map_err(|e| e.to_string())
}

fn lock(env: &mut JNIEnv, app: &AndroidApp) -> Result<(), jni::errors::Error> {
    // SAFETY: activity_as_ptr is the live Activity reference owned by android-activity; JObject does
    // not free it on drop.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
    // The application context, so the lock does not hang on the activity.
    let ctx = env.call_method(&activity, "getApplicationContext", "()Landroid/content/Context;", &[])?.l()?;
    let service = env.new_string("wifi")?;
    let wifi = env.call_method(&ctx, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&service)])?.l()?;
    if wifi.is_null() {
        return Err(jni::errors::Error::NullPtr("no WifiManager"));
    }
    let tag = env.new_string("tonestrand")?;
    let lock = env
        .call_method(
            &wifi,
            "createMulticastLock",
            "(Ljava/lang/String;)Landroid/net/wifi/WifiManager$MulticastLock;",
            &[JValue::Object(&tag)],
        )?
        .l()?;
    env.call_method(&lock, "setReferenceCounted", "(Z)V", &[JValue::Bool(0)])?;
    env.call_method(&lock, "acquire", "()V", &[])?;
    // Keep a global reference for the life of the process: the lock's finalizer releases it if the Java object is collected.
    std::mem::forget(env.new_global_ref(&lock)?);
    Ok(())
}
