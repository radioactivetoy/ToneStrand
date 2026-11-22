//! Files on Android: the system picker, the shared Documents/ToneStrand folder and the storage
//! permission. The Java side is `MainActivity` (app/app/src/main/java/com/tonestrand/app); this
//! file installs the `tonestrand::platform` backends that call it, and implements the native
//! methods it calls back (`Java_com_tonestrand_app_MainActivity_*`, resolved by name because the
//! library is loaded by GameActivity).
//!
//! Backends run on the render thread; the native methods run on the UI thread or a Java worker
//! thread and only call `deliver` / `set_storage` (short locks, no JNI back into Java).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{GlobalRef, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jlong};
use jni::{JNIEnv, JavaVM};
use tonestrand::platform::{self, PickKind, PickRequest, PickResult, Storage};
use winit::platform::android::activity::AndroidApp;

struct Host {
    vm: JavaVM,
    activity: GlobalRef,
}

/// The current activity; replaced by each `init` so a new activity in the same process is used.
static HOST: Mutex<Option<Arc<Host>>> = Mutex::new(None);
/// `<Documents>/ToneStrand`, when the Java side could tell.
static SHARED: OnceLock<PathBuf> = OnceLock::new();
/// The app's cache folder (picked files and files waiting to be saved).
static CACHE: OnceLock<PathBuf> = OnceLock::new();
/// The suggested name of the pending save request (one request at a time).
static SUGGESTED: Mutex<Option<(u64, String)>> = Mutex::new(None);

const FOLDER: &str = "ToneStrand";

/// Runs `f` on the activity with this thread attached; pending Java exceptions are cleared.
fn with_activity<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Result<R, String> {
    let host = HOST.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("not started")?;
    let mut env = host.vm.attach_current_thread().map_err(|e| e.to_string())?;
    let res = env.with_local_frame(32, |env| f(env, host.activity.as_obj()));
    if res.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    res.map_err(|e| e.to_string())
}

fn call_string(env: &mut JNIEnv, activity: &JObject, method: &str) -> jni::errors::Result<String> {
    let s: JString = env.call_method(activity, method, "()Ljava/lang/String;", &[])?.l()?.into();
    let out: String = env.get_string(&s)?.into();
    Ok(out)
}

fn sdk_int(env: &mut JNIEnv) -> jni::errors::Result<i32> {
    env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()
}

/// Whether files can be created in `dir` (it is created if needed).
fn usable(dir: &Path) -> bool {
    let probe = dir.join(".tonestrand-write-test");
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(&probe, b"x").is_ok() && std::fs::remove_file(&probe).is_ok()
}

/// Sets up everything the app needs before `run_native`: the library folders (environment), the
/// storage state, the cache folder, and the picker, commit and permission backends.
pub fn init(app: &AndroidApp, private_library: &Path) {
    std::env::set_var("TONESTRAND_LIBRARY_PRIVATE", private_library);
    std::env::set_var("TONESTRAND_LIBRARY", private_library);
    if let Err(e) = try_init(app, private_library) {
        log::error!("files: {e}");
    }
}

fn try_init(app: &AndroidApp, private_library: &Path) -> Result<(), String> {
    // SAFETY: vm_as_ptr is the process's JavaVM and activity_as_ptr the live activity, both owned
    // by android-activity for the life of the app; the activity is kept as a global reference.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) }.map_err(|e| e.to_string())?;
    let activity = {
        let env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let obj = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        env.new_global_ref(&obj).map_err(|e| e.to_string())?
    };
    *HOST.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Host { vm, activity }));

    let (api, cache, documents) = with_activity(|env, a| Ok((sdk_int(env)?, call_string(env, a, "cacheDir")?, call_string(env, a, "sharedDocumentsDir")?)))?;
    platform::set_android_api(api.max(0) as u32);
    let cache = PathBuf::from(cache);
    let _ = std::fs::create_dir_all(&cache);
    let _ = CACHE.set(cache.clone());
    platform::set_cache_dir(cache);

    let shared = PathBuf::from(documents).join(FOLDER);
    let _ = SHARED.set(shared.clone());
    if usable(&shared) {
        log::info!("shared folder {} is usable", shared.display());
        std::env::set_var("TONESTRAND_LIBRARY", &shared);
        platform::set_storage(Storage::Shared(shared));
    } else {
        log::info!("shared folder {} is not usable yet", shared.display());
        std::env::set_var("TONESTRAND_LIBRARY", private_library);
        platform::set_storage(Storage::Private);
    }

    platform::set_backend(Box::new(pick));
    platform::set_commit_backend(Box::new(|id, path| {
        let temp = path.to_string_lossy().into_owned();
        let res = with_activity(|env, a| {
            let temp = env.new_string(&temp)?;
            env.call_method(a, "commitSave", "(JLjava/lang/String;)V", &[JValue::Long(id as jlong), JValue::Object(&temp)])?;
            Ok(())
        });
        if let Err(e) = res {
            platform::abort_save(id, e);
        }
    }));
    platform::set_permission_backend(Box::new(|| {
        if let Err(e) = with_activity(|env, a| env.call_method(a, "requestStorage", "()V", &[]).map(|_| ())) {
            log::error!("requestStorage: {e}");
        }
    }));
    Ok(())
}

/// The picker backend: starts the system picker through the activity.
fn pick(req: PickRequest) {
    let id = req.id;
    let res = match req.kind {
        PickKind::Open { multiple } => with_activity(|env, a| {
            let string = env.find_class("java/lang/String")?;
            let types = env.new_object_array(1, string, JObject::null())?;
            let any = env.new_string("*/*")?;
            env.set_object_array_element(&types, 0, &any)?;
            env.call_method(
                a,
                "pickOpen",
                "(JZ[Ljava/lang/String;)V",
                &[JValue::Long(id as jlong), JValue::Bool(multiple as jboolean), JValue::Object(&types)],
            )
            .map(|_| ())
        }),
        PickKind::Save { suggested } => {
            *SUGGESTED.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, suggested.clone()));
            with_activity(|env, a| {
                let name = env.new_string(&suggested)?;
                env.call_method(a, "pickSave", "(JLjava/lang/String;)V", &[JValue::Long(id as jlong), JValue::Object(&name)]).map(|_| ())
            })
        }
        PickKind::Folder => Err("Not available on Android".to_string()),
    };
    if let Err(e) = res {
        log::error!("picker: {e}");
        platform::deliver(id, PickResult::Failed(e));
    }
}

fn string_of(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).ok().map(Into::into)
}

/// A file name that is safe to create inside a folder of ours.
fn clean_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    if n.is_empty() || n == "." || n == ".." {
        "Untitled".into()
    } else {
        n
    }
}

#[no_mangle]
pub extern "system" fn Java_com_tonestrand_app_MainActivity_nativeCreated(_env: JNIEnv, _class: JClass) {
    platform::cancel_pending();
}

#[no_mangle]
pub extern "system" fn Java_com_tonestrand_app_MainActivity_nativePicked(mut env: JNIEnv, _class: JClass, id: jlong, paths: JObjectArray) {
    let mut files = Vec::new();
    let n = if paths.is_null() { 0 } else { env.get_array_length(&paths).unwrap_or(0) };
    for i in 0..n {
        if let Ok(o) = env.get_object_array_element(&paths, i) {
            if let Some(s) = string_of(&mut env, &JString::from(o)) {
                files.push(PathBuf::from(s));
            }
        }
    }
    let result = if files.is_empty() { PickResult::Cancelled } else { PickResult::Files(files) };
    platform::deliver(id as u64, result);
}

#[no_mangle]
pub extern "system" fn Java_com_tonestrand_app_MainActivity_nativePickFailed(mut env: JNIEnv, _class: JClass, id: jlong, error: JString) {
    let e = string_of(&mut env, &error).unwrap_or_else(|| "Could not open the file".into());
    platform::deliver(id as u64, PickResult::Failed(e));
}

#[no_mangle]
pub extern "system" fn Java_com_tonestrand_app_MainActivity_nativeSaveTarget(_env: JNIEnv, _class: JClass, id: jlong) {
    let id = id as u64;
    let suggested = match SUGGESTED.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some((i, s)) if i == id => s,
        _ => "Untitled".into(),
    };
    let result = match CACHE.get() {
        Some(cache) => {
            let dir = cache.join(format!("save-{id}"));
            match std::fs::create_dir_all(&dir) {
                Ok(()) => PickResult::SaveTo(dir.join(clean_name(&suggested))),
                Err(e) => PickResult::Failed(e.to_string()),
            }
        }
        None => PickResult::Failed("No cache folder".into()),
    };
    platform::deliver(id, result);
}

#[no_mangle]
pub extern "system" fn Java_com_tonestrand_app_MainActivity_nativeSaved(mut env: JNIEnv, _class: JClass, id: jlong, error: JString) {
    match string_of(&mut env, &error) {
        None => platform::deliver(id as u64, PickResult::Saved),
        Some(e) => platform::abort_save(id as u64, e),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_tonestrand_app_MainActivity_nativeStorageChanged(_env: JNIEnv, _class: JClass, granted: jboolean) {
    // The folder test touches the disk: not on the UI thread.
    std::thread::spawn(move || {
        let storage = match SHARED.get() {
            Some(dir) if granted != 0 && usable(dir) => Storage::Shared(dir.clone()),
            _ => Storage::Refused,
        };
        log::info!("storage access: {storage:?}");
        platform::set_storage(storage);
    });
}
