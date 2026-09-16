#[cfg(target_os = "android")]
use vpnhide_checks::run_all_json;

#[cfg(target_os = "android")]
const LOG_TAG: &str = "VpnHide-Native";

#[cfg(target_os = "android")]
unsafe extern "C" {
    fn __android_log_write(
        prio: libc::c_int,
        tag: *const libc::c_char,
        text: *const libc::c_char,
    ) -> libc::c_int;
}

#[cfg(target_os = "android")]
fn log_error(message: &str) {
    use std::ffi::CString;

    const ANDROID_LOG_ERROR: libc::c_int = 6;
    let (Ok(tag), Ok(text)) = (CString::new(LOG_TAG), CString::new(message)) else {
        return;
    };
    // SAFETY: both pointers are NUL-terminated and outlive the call.
    unsafe {
        __android_log_write(ANDROID_LOG_ERROR, tag.as_ptr(), text.as_ptr());
    }
}

#[cfg(target_os = "android")]
fn install_panic_hook() {
    use std::sync::Once;

    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log_error(&format!("panic in native probe: {info}"));
            previous(info);
        }));
    });
}

/// In-process app-view entry. The app-native profile deliberately uses unwind;
/// jni's resolver catches a panic and turns it into a Java exception.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_okhsunrog_vpnhide_checks_NativeProbe_runAllChecksJson<'local>(
    mut env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
) -> jni::objects::JString<'local> {
    install_panic_hook();
    env.with_env(
        |env| -> jni::errors::Result<jni::objects::JString<'local>> {
            env.new_string(run_all_json())
        },
    )
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
