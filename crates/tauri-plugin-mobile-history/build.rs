fn main() {
    tauri_plugin::Builder::new(&[
        "open",
        "update",
        "close",
        "register_listener",
        "registerListener",
        "remove_listener",
        "removeListener",
    ])
    .android_path("android")
    .build();
}
