//! Confined JNI bridge between Rust and `ai.storyteller.figcraft.FigCraftActivity`.
//!
//! All `unsafe` FFI code in `figcraft-android` lives in this module.

use crate::protocol::{ActivityEvent, BridgeReply};

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use android_activity::AndroidApp;
    use jni::objects::{GlobalRef, JObject, JString, JValue};
    use jni::JavaVM;
    use std::sync::OnceLock;

    struct AndroidBridge {
        vm: JavaVM,
        activity: GlobalRef,
    }

    static BRIDGE: OnceLock<AndroidBridge> = OnceLock::new();

    pub fn init(app: &AndroidApp) -> Result<(), String> {
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|e| format!("JavaVM::from_raw failed: {e}"))?;
        let env = vm
            .attach_current_thread()
            .map_err(|e| format!("attach_current_thread failed: {e}"))?;
        let activity_obj = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let global_ref = env
            .new_global_ref(activity_obj)
            .map_err(|e| format!("new_global_ref failed: {e}"))?;

        let _ = BRIDGE.set(AndroidBridge {
            vm,
            activity: global_ref,
        });
        Ok(())
    }

    pub fn call(cmd: &str, arg: &str) -> BridgeReply {
        let Some(bridge) = BRIDGE.get() else {
            return BridgeReply::Err("bridge not initialized".into());
        };
        let Ok(mut env) = bridge.vm.attach_current_thread() else {
            return BridgeReply::Err("failed to attach JNI thread".into());
        };
        let Ok(j_cmd) = env.new_string(cmd) else {
            return BridgeReply::Err("failed to allocate jstring cmd".into());
        };
        let Ok(j_arg) = env.new_string(arg) else {
            return BridgeReply::Err("failed to allocate jstring arg".into());
        };

        let result = env.call_method(
            bridge.activity.as_obj(),
            "figcraftCall",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&j_cmd), JValue::Object(&j_arg)],
        );

        match result {
            Ok(val) => match val.l() {
                Ok(obj) => {
                    let jstr: JString = obj.into();
                    match env.get_string(&jstr) {
                        Ok(s) => BridgeReply::parse(&s.to_string_lossy()),
                        Err(e) => BridgeReply::Err(format!("get_string failed: {e}")),
                    }
                }
                Err(e) => BridgeReply::Err(format!("invalid return object: {e}")),
            },
            Err(e) => {
                let _ = env.exception_clear();
                BridgeReply::Err(format!("JNI call_method failed: {e}"))
            }
        }
    }

    pub fn poll_events() -> Vec<ActivityEvent> {
        let mut events = Vec::new();
        for _ in 0..64 {
            match call("poll", "") {
                BridgeReply::Ok(Some(line)) if !line.is_empty() => {
                    if let Some(ev) = ActivityEvent::parse(&line) {
                        events.push(ev);
                    }
                }
                _ => break,
            }
        }
        events
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;

    pub fn call(_cmd: &str, _arg: &str) -> BridgeReply {
        BridgeReply::Ok(None)
    }

    pub fn poll_events() -> Vec<ActivityEvent> {
        Vec::new()
    }
}

#[cfg(target_os = "android")]
pub fn init_android_bridge(app: &android_activity::AndroidApp) -> Result<(), String> {
    imp::init(app)
}

pub fn call_activity(cmd: &str, arg: &str) -> BridgeReply {
    imp::call(cmd, arg)
}

pub fn poll_activity_events() -> Vec<ActivityEvent> {
    imp::poll_events()
}
