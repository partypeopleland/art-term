// Desktop continues to use its existing binary entry point. Android requires a
// shared library entry point loaded by its Activity.
#[cfg(target_os = "android")]
mod android_operator;
#[cfg(target_os = "android")]
mod mobile;
#[cfg(target_os = "android")]
mod runtime_bootstrap;
#[cfg(target_os = "android")]
mod runtime_hosts;
#[cfg(target_os = "android")]
mod runtime_sessions;
#[cfg(target_os = "android")]
mod runtime_transfer;

#[cfg(target_os = "android")]
#[tauri::mobile_entry_point]
pub fn run() {
    mobile::run();
}
