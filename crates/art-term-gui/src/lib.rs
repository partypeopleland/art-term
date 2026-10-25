// Desktop continues to use its existing binary entry point. Android requires a
// shared library entry point loaded by its Activity.
#[cfg(target_os = "android")]
#[path = "platform/android/android_operator.rs"]
mod android_operator;
#[cfg(target_os = "android")]
#[path = "shared/instance_desktops.rs"]
mod instance_desktops;
#[cfg(target_os = "android")]
#[path = "shared/instance_restore.rs"]
mod instance_restore;
#[cfg(target_os = "android")]
#[path = "platform/android/mobile.rs"]
mod mobile;
#[cfg(target_os = "android")]
#[path = "shared/runtime_bootstrap.rs"]
mod runtime_bootstrap;
#[cfg(target_os = "android")]
#[path = "shared/runtime_hosts.rs"]
mod runtime_hosts;
#[cfg(target_os = "android")]
#[path = "shared/runtime_sessions.rs"]
mod runtime_sessions;
#[cfg(target_os = "android")]
#[path = "shared/runtime_transfer.rs"]
mod runtime_transfer;

#[cfg(target_os = "android")]
#[tauri::mobile_entry_point]
pub fn run() {
    mobile::run();
}
