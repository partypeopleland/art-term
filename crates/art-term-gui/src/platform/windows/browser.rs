//! Open only validated web URLs through the local OS browser association.
fn validate(url: &str) -> Result<(), &'static str> {
    if url.is_empty() || url.len() > 8192 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Invalid web URL");
    }
    let (scheme, rest) = url.split_once("://").ok_or("Invalid web URL")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || rest.split(['/', '?', '#']).next().unwrap_or("").is_empty() {
        return Err("Invalid web URL");
    }
    let parsed = url::Url::parse(url).map_err(|_| "Invalid web URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() { return Err("Invalid web URL"); }
    Ok(())
}
fn launch_with(url: &str, opener: impl FnOnce(Vec<u16>) -> Result<isize, &'static str>) -> Result<(), &'static str> {
    validate(url)?;
    let wide = url.encode_utf16().chain(std::iter::once(0)).collect();
    match opener(wide)? { code if code > 32 => Ok(()), _ => Err("Unable to open system browser") }
}
pub fn open_web_url(url: &str) -> Result<(), &'static str> { launch_with(url, native_open) }
#[cfg(not(windows))]
fn native_open(_: Vec<u16>) -> Result<isize, &'static str> { Err("System browser unavailable") }
#[cfg(windows)]
fn native_open(url: Vec<u16>) -> Result<isize, &'static str> {
    // A dedicated STA avoids changing the Tauri caller's existing COM apartment.
    std::thread::spawn(move || {
        use std::ffi::c_void;
        #[link(name = "ole32")]
        extern "system" {
            fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
            fn CoUninitialize();
        }
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(window: *mut c_void, operation: *const u16, file: *const u16,
                parameters: *const u16, directory: *const u16, show: i32) -> *mut c_void;
        }
        struct Com;
        impl Drop for Com { fn drop(&mut self) { unsafe { CoUninitialize() } } }
        unsafe {
            // COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE.
            if CoInitializeEx(std::ptr::null_mut(), 0x2 | 0x4) < 0 { return Err("System browser initialization failed"); }
            let _com = Com;
            let open: [u16; 5] = [111,112,101,110,0];
            Ok(ShellExecuteW(std::ptr::null_mut(), open.as_ptr(), url.as_ptr(),
                std::ptr::null(), std::ptr::null(), 1) as isize)
        }
    }).join().map_err(|_| "System browser initialization failed")?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_boundary_rejects_unsafe_input_without_calling_launcher() {
        for url in ["", "javascript:alert(1)", "file:///C:/a", "https://", "https:///example.org", "https:example.org", "https://e.test/\nnext", "https://e.test/\0", "https://e.test/a b", "https://e.test/\u{7f}"] {
            assert!(launch_with(url, |_| panic!("invalid URL reached launcher")).is_err(), "{url:?}");
        }
        let long = format!("https://e.test/{}", "x".repeat(8192));
        assert!(launch_with(&long, |_| panic!("oversize reached launcher")).is_err());
    }
    #[test]
    fn browser_boundary_preserves_original_url_and_opens_exactly_once() {
        for url in ["http://example.org/a?x=1&y=2#fragment", "HTTPS://example.org/測試?q=%E4%B8%AD#文"] {
            let calls = std::cell::Cell::new(0);
            launch_with(url, |wide| {
                calls.set(calls.get()+1); assert_eq!(wide.last(),Some(&0));
                assert_eq!(String::from_utf16(&wide[..wide.len()-1]).unwrap(),url); Ok(33)
            }).unwrap();
            assert_eq!(calls.get(),1);
        }
    }
    #[test]
    fn browser_boundary_surfaces_launch_errors_and_checks_return_32() {
        for code in [0,2,31,32] { assert!(launch_with("https://example.org", |_| Ok(code)).is_err()); }
        assert!(launch_with("https://example.org", |_| Ok(33)).is_ok());
        assert_eq!(launch_with("https://example.org", |_| Err("injected failure")),Err("injected failure"));
    }
}
