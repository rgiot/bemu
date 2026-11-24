//! The functions of `org.bemu.droid.Native` (Kotlin `external fun`s).
//!
//! An emulator is a handle (a `Long` for Kotlin): its emulation thread's command channel and the
//! touch input it reads. Calls that answer (`load`, `configure`) wait for the thread.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, Once};
use std::sync::TryLockError;

use jni::objects::{JByteArray, JObject, JString};
use jni::sys::{jboolean, jint, jlong, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use ndk::native_window::NativeWindow;

use crate::config::Config;
use crate::machine::Input;
use crate::runner::{self, Command};

struct Handle {
    commands: Mutex<Sender<Command>>,
    input: Arc<Input>,
}

impl Handle {
    fn send(&self, command: Command) {
        if let Ok(commands) = self.commands.lock() {
            let _ = commands.send(command);
        }
    }
}

/// # Safety
/// `handle` comes from [`Java_org_bemu_droid_Native_create`] and was not destroyed.
unsafe fn handle<'a>(handle: jlong) -> Option<&'a Handle> {
    (handle as *const Handle).as_ref()
}

fn string(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(String::from).unwrap_or_default()
}

fn jstring(env: &mut JNIEnv, s: &str) -> jni::sys::jstring {
    env.new_string(s).map(JString::into_raw).unwrap_or(std::ptr::null_mut())
}

/// Latest performance snapshot; reading it does not wait for the emulation thread.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_diagnostics(
    mut env: JNIEnv,
    _this: JObject,
    raw_handle: jlong,
) -> jni::sys::jstring {
    let Some(handle) = (unsafe { handle(raw_handle) }) else {
        return jstring(&mut env, "Emulator unavailable");
    };
    match handle.input.diagnostics.try_lock() {
        Ok(report) if report.is_empty() => jstring(&mut env, "Collecting measurements..."),
        Ok(report) => jstring(&mut env, &report),
        Err(TryLockError::WouldBlock) => jstring(&mut env, "Updating measurements..."),
        Err(TryLockError::Poisoned(error)) => {
            log::error!("cannot read diagnostics: {error}");
            jstring(&mut env, "Diagnostics unavailable")
        }
    }
}

static INIT: Once = Once::new();

/// The logger, the Android context (AAudio through cpal), the system's certificates (the M4's
/// HTTPS, read by rustls-native-certs).
fn init(env: &mut JNIEnv, context: &JObject) {
    INIT.call_once(|| {
        android_logger::init_once(android_logger::Config::default().with_tag("bemu").with_max_level(log::LevelFilter::Info));
        std::panic::set_hook(Box::new(|info| log::error!("{info}")));
        if std::env::var_os("SSL_CERT_DIR").is_none() {
            std::env::set_var("SSL_CERT_DIR", "/system/etc/security/cacerts");
        }
        if let (Ok(vm), Ok(context)) = (env.get_java_vm(), env.new_global_ref(context)) {
            // kept for the process's life
            let context = Box::leak(Box::new(context));
            unsafe {
                ndk_context::initialize_android_context(vm.get_java_vm_pointer().cast(), context.as_obj().as_raw().cast());
            }
        }
    });
}

/// `create(context, settingsJson, sdFolder, stateFolder): Long` — 0 when the machine cannot be
/// built (see the log). The machine saved in `stateFolder` comes back if its settings are these.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_create(
    mut env: JNIEnv,
    _this: JObject,
    context: JObject,
    settings: JString,
    sd: JString,
    state: JString,
) -> jlong {
    init(&mut env, &context);
    let settings = string(&mut env, &settings);
    let sd = string(&mut env, &sd);
    let state = string(&mut env, &state);
    let config = match Config::from_json(&settings) {
        Ok(config) => config,
        Err(e) => {
            log::warn!("{e}: the defaults instead");
            Config::default()
        }
    };
    let input = Arc::new(Input::default());
    match runner::spawn(config, sd.into(), state.into(), Arc::clone(&input)) {
        Ok(commands) => Box::into_raw(Box::new(Handle {
            commands: Mutex::new(commands),
            input,
        })) as jlong,
        Err(e) => {
            log::error!("{e}");
            0
        }
    }
}

/// `destroy(handle)`: the emulation thread stops.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_destroy(_env: JNIEnv, _this: JObject, handle: jlong) {
    if handle != 0 {
        let handle = unsafe { Box::from_raw(handle as *mut Handle) };
        handle.send(Command::Quit);
    }
}

/// `setSurface(handle, surface?)`: drawn in `surface` (null: it is being destroyed; returns once
/// nothing draws in it any more).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_setSurface(mut env: JNIEnv, _this: JObject, handle: jlong, surface: JObject) {
    let Some(handle) = (unsafe { self::handle(handle) }) else { return };
    let window = if surface.is_null() {
        None
    } else {
        unsafe { NativeWindow::from_surface(env.get_raw().cast(), surface.as_raw().cast()) }
    };
    let (done, wait) = channel();
    handle.send(Command::Surface(window, done));
    let _ = wait.recv();
    let _ = &mut env;
}

/// `resize(handle, width, height)`: the surface's new size, in pixels.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_resize(_env: JNIEnv, _this: JObject, handle: jlong, width: jint, height: jint) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.send(Command::Resize(width.max(1) as u32, height.max(1) as u32));
    }
}

/// `pause(handle, paused)`: the app goes to the background (or comes back).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_pause(_env: JNIEnv, _this: JObject, handle: jlong, paused: jboolean) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.input.release_keys();
        handle.send(Command::Pause(paused != JNI_FALSE));
    }
}

/// `load(handle, name, bytes): String` — `"ok:"` and what autorun types, or `"error:"` and why.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_load(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    name: JString,
    bytes: JByteArray,
) -> jni::sys::jstring {
    let Some(handle) = (unsafe { self::handle(handle) }) else {
        return jstring(&mut env, "error:no emulator");
    };
    let name = string(&mut env, &name);
    let bytes = env.convert_byte_array(&bytes).unwrap_or_default();
    let (reply, answer) = channel();
    handle.send(Command::Load(name, bytes, reply));
    let answer = match answer.recv() {
        Ok(Ok(typed)) => format!("ok:{}", typed.unwrap_or_default()),
        Ok(Err(e)) => format!("error:{e}"),
        Err(_) => "error:the emulator stopped".to_owned(),
    };
    jstring(&mut env, &answer)
}

/// `reset(handle)`: switched off and on, its medium started again.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_reset(_env: JNIEnv, _this: JObject, handle: jlong) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.send(Command::Reset);
    }
}

/// `configure(handle, settingsJson): String?` — null, or why the settings were refused.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_configure(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    settings: JString,
) -> jni::sys::jstring {
    let Some(handle) = (unsafe { self::handle(handle) }) else {
        return jstring(&mut env, "no emulator");
    };
    let settings = string(&mut env, &settings);
    let error = match Config::from_json(&settings) {
        Ok(config) => {
            let (reply, answer) = channel();
            handle.send(Command::Configure(config, reply));
            answer.recv().unwrap_or(Ok(())).err()
        }
        Err(e) => Some(e),
    };
    match error {
        Some(e) => jstring(&mut env, &e),
        None => std::ptr::null_mut(),
    }
}

/// `defaultSettings(): String`: the settings' JSON, defaults.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_defaultSettings(mut env: JNIEnv, _this: JObject) -> jni::sys::jstring {
    let json = serde_json::to_string(&Config::default()).unwrap_or_default();
    jstring(&mut env, &json)
}

/// `typeText(handle, text)`: typed on the CPC's keyboard (CSL escapes: `\(RET)`...).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_typeText(mut env: JNIEnv, _this: JObject, handle: jlong, text: JString) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        let text = string(&mut env, &text);
        handle.send(Command::TypeText(text));
    }
}

/// `key(handle, number, down)`: the key of the firmware's number (matrix line * 8 + bit).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_key(_env: JNIEnv, _this: JObject, handle: jlong, number: jint, down: jboolean) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.input.key(number.clamp(0, 79) as u8, down != JNI_FALSE);
    }
}

/// `releaseKeys(handle)`: every key up.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_releaseKeys(_env: JNIEnv, _this: JObject, handle: jlong) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.input.release_keys();
    }
}

/// `joystick(handle, bits)`: 1 up, 2 down, 4 left, 8 right, 16 fire 1, 32 fire 2.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_joystick(_env: JNIEnv, _this: JObject, handle: jlong, bits: jint) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.input.joystick(bits as u8);
    }
}

/// `hasCrt(handle): Boolean`: whether the GPU draws the CRT (else the picture is raw).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_hasCrt(_env: JNIEnv, _this: JObject, handle: jlong) -> jboolean {
    match unsafe { self::handle(handle) } {
        Some(handle) if handle.input.crt.load(std::sync::atomic::Ordering::Relaxed) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

/// `keyLegends(language): String`: the keycaps of the CPC's keyboard in `language` ("en" or "fr"):
/// 80 lines (the firmware's key numbers), each "normal\tshifted", or empty for a key that types no
/// printable character. Taken from the firmware's own key tables.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_keyLegends(mut env: JNIEnv, _this: JObject, language: JString) -> jni::sys::jstring {
    let layout = match string(&mut env, &language).as_str() {
        "fr" => amsrust::keyboard::KeyboardLayout::French,
        _ => amsrust::keyboard::KeyboardLayout::Uk,
    };
    let lines: Vec<String> = (0..80)
        .map(|index| layout.legend(index).map_or(String::new(), |(normal, shifted)| format!("{normal}\t{shifted}")))
        .collect();
    jstring(&mut env, &lines.join("\n"))
}

/// `save(handle): String?` — the machine saved (the app going to the background, which Android
/// may end): null, or why it could not be.
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_save(mut env: JNIEnv, _this: JObject, handle: jlong) -> jni::sys::jstring {
    let Some(handle) = (unsafe { self::handle(handle) }) else {
        return std::ptr::null_mut();
    };
    let (reply, answer) = channel();
    handle.send(Command::Save(reply));
    match answer.recv() {
        Ok(Err(e)) => jstring(&mut env, &e),
        _ => std::ptr::null_mut(),
    }
}

/// `takeCrtDropped(handle): Boolean`: whether the phone was found too slow for the CRT since the
/// last call (the picture is raw now: the app says so).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_takeCrtDropped(_env: JNIEnv, _this: JObject, handle: jlong) -> jboolean {
    match unsafe { self::handle(handle) } {
        Some(handle) if handle.input.crt_dropped.swap(false, std::sync::atomic::Ordering::Relaxed) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

/// `keepCrt(handle)`: the CRT back, too slow as it is (the user asked).
#[no_mangle]
pub extern "system" fn Java_org_bemu_droid_Native_keepCrt(_env: JNIEnv, _this: JObject, handle: jlong) {
    if let Some(handle) = unsafe { self::handle(handle) } {
        handle.send(Command::KeepCrt);
    }
}
