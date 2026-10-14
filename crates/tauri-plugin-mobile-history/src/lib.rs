use tauri::{plugin::TauriPlugin, Runtime};

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("mobile-history")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _api.register_android_plugin("net.missart.artterm.history", "MobileHistoryPlugin")?;
            Ok(())
        })
        .build()
}
