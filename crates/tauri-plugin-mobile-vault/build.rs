fn main() {
    // Generate an ACL manifest even though no native vault command is granted
    // to the WebView. Rust invokes these through the private plugin handle.
    tauri_plugin::Builder::new(&["operatorInitialize", "operatorSeal", "operatorUnseal"])
        .android_path("android")
        .build();
}
