fn main() {
    tauri_plugin::Builder::new(&[
        "open",
        "openUrl",
        "update",
        "close",
        "keyboardStatus",
        "showKeyboard",
        "register_listener",
        "registerListener",
        "remove_listener",
        "removeListener",
    ])
    .android_path("android")
    .build();
}
