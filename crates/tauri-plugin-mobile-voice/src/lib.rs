use tauri::{plugin::TauriPlugin, Runtime};

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("mobile-voice")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let _plugin =
                    _api.register_android_plugin("net.missart.artterm.voice", "MobileVoicePlugin")?;
                let _ = _plugin;
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    #[test]
    fn voice_grants_are_android_only() {
        // The WebView grant lives in the Android-only capability; the desktop
        // capability must never reference this plugin.
        let desktop_capability = include_str!("../../art-term-gui/capabilities/main.json");
        assert!(!desktop_capability.contains("mobile-voice"));
        let android_config = include_str!("../../art-term-gui/tauri.android.conf.json");
        assert!(android_config.contains("mobile-voice:default"));
    }
}
