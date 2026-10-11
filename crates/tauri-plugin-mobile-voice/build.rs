fn main() {
    // registerListener variants keep both JS spellings reachable
    // (addPluginListener tries register_listener first, then registerListener).
    tauri_plugin::Builder::new(&[
        "status",
        "requestPermission",
        "downloadLanguage",
        "start",
        "stop",
        "cancel",
        "register_listener",
        "registerListener",
        "remove_listener",
        "removeListener",
    ])
    .android_path("android")
    .build();
}
