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
    use std::sync::RwLock;

    struct AndroidBridge {
        app: AndroidApp,
        vm: JavaVM,
        activity: GlobalRef,
    }

    static BRIDGE: RwLock<Option<AndroidBridge>> = RwLock::new(None);

    pub fn init(app: &AndroidApp) -> Result<(), String> {
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|e| format!("JavaVM::from_raw failed: {e}"))?;
        let global_ref = {
            let env = vm
                .attach_current_thread()
                .map_err(|e| format!("attach_current_thread failed: {e}"))?;
            let activity_obj = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
            env.new_global_ref(activity_obj)
                .map_err(|e| format!("new_global_ref failed: {e}"))?
        };

        if let Ok(mut guard) = BRIDGE.write() {
            *guard = Some(AndroidBridge {
                app: app.clone(),
                vm,
                activity: global_ref,
            });
        }
        Ok(())
    }

    pub fn call(cmd: &str, arg: &str) -> BridgeReply {
        let Ok(guard) = BRIDGE.read() else {
            return BridgeReply::Err("bridge lock poisoned".into());
        };
        let Some(bridge) = guard.as_ref() else {
            return BridgeReply::Err("bridge not initialized".into());
        };
        let Ok(mut env) = bridge.vm.attach_current_thread() else {
            return BridgeReply::Err("failed to attach JNI thread".into());
        };

        let frame_res: Result<BridgeReply, jni::errors::Error> =
            env.with_local_frame(16, |env| {
                let j_cmd = env.new_string(cmd)?;
                let j_arg = env.new_string(arg)?;

                let result = env.call_method(
                    bridge.activity.as_obj(),
                    "figcraftCall",
                    "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&j_cmd), JValue::Object(&j_arg)],
                );

                let reply = match result {
                    Ok(val) => match val.l() {
                        Ok(obj) if !obj.is_null() => {
                            let jstr: JString = obj.into();
                            match env.get_string(&jstr) {
                                Ok(s) => BridgeReply::parse(&s.to_string_lossy()),
                                Err(e) => BridgeReply::Err(format!("get_string failed: {e}")),
                            }
                        }
                        Ok(_) => BridgeReply::Ok(None),
                        Err(e) => BridgeReply::Err(format!("invalid return object: {e}")),
                    },
                    Err(e) => {
                        let _ = env.exception_clear();
                        BridgeReply::Err(format!("JNI call_method failed: {e}"))
                    }
                };
                Ok(reply)
            });

        match frame_res {
            Ok(reply) => reply,
            Err(e) => {
                let _ = env.exception_clear();
                BridgeReply::Err(format!("JNI local frame failed: {e}"))
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

    pub fn game_show_soft_input(show: bool) {
        let Ok(guard) = BRIDGE.read() else { return; };
        let Some(bridge) = guard.as_ref() else { return; };
        if show {
            bridge.app.set_text_input_state(android_activity::input::TextInputState {
                text: " ".to_owned(),
                selection: android_activity::input::TextSpan { start: 1, end: 1 },
                compose_region: None,
            });
            bridge.app.show_soft_input(true);
        } else {
            bridge.app.hide_soft_input(false);
        }
    }

    pub fn game_reset_text_input() {
        let Ok(guard) = BRIDGE.read() else { return; };
        let Some(bridge) = guard.as_ref() else { return; };
        bridge.app.set_text_input_state(android_activity::input::TextInputState {
            text: " ".to_owned(),
            selection: android_activity::input::TextSpan { start: 1, end: 1 },
            compose_region: None,
        });
    }

    pub fn game_poll_text_input() -> Option<(String, Option<(usize, usize)>)> {
        let Ok(guard) = BRIDGE.read() else { return None; };
        let Some(bridge) = guard.as_ref() else { return None; };
        let st = bridge.app.text_input_state();
        Some((st.text, st.compose_region.map(|r| (r.start, r.end))))
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

    pub fn game_show_soft_input(_show: bool) {}
    pub fn game_reset_text_input() {}
    pub fn game_poll_text_input() -> Option<(String, Option<(usize, usize)>)> {
        None
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

pub fn game_show_soft_input(show: bool) {
    imp::game_show_soft_input(show);
}

pub fn game_reset_text_input() {
    imp::game_reset_text_input();
}

pub fn game_poll_text_input() -> Option<(String, Option<(usize, usize)>)> {
    imp::game_poll_text_input()
}
