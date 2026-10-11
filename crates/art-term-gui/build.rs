fn main() {
    for name in [
        "ART_TERM_ENABLE_PRIVATE_RUNTIME_BOOTSTRAP",
        "ART_TERM_PRIVATE_RUNTIME_BOOTSTRAP_FILE",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        assert!(
            std::env::var_os(name).is_none_or(|v| v.is_empty()),
            "private runtime bootstrap is retired; create or import connections in the App"
        );
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("runtime_bootstrap.rs"), "pub(crate) const PRIVATE_RUNTIME_BOOTSTRAP: Option<crate::runtime_bootstrap::Seed> = None;\n").expect("cannot write retired bootstrap declaration");
    tauri_build::build()
}
