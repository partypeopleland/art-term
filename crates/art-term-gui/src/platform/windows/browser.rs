use super::WebUrl;

fn launch_with(
    url: WebUrl<'_>,
    opener: impl FnOnce(Vec<u16>) -> Result<isize, &'static str>,
) -> Result<(), &'static str> {
    let wide = url.0.encode_utf16().chain(std::iter::once(0)).collect();
    match opener(wide)? {
        code if code > 32 => Ok(()),
        _ => Err("Unable to open system browser"),
    }
}

pub(super) fn open(url: WebUrl<'_>) -> Result<(), &'static str> {
    launch_with(url, native_open)
}

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
            fn ShellExecuteW(
                window: *mut c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> *mut c_void;
        }
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe { CoUninitialize() }
            }
        }
        unsafe {
            // COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE.
            if CoInitializeEx(std::ptr::null_mut(), 0x2 | 0x4) < 0 {
                return Err("System browser initialization failed");
            }
            let _com = Com;
            let open: [u16; 5] = [111, 112, 101, 110, 0];
            Ok(ShellExecuteW(
                std::ptr::null_mut(),
                open.as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            ) as isize)
        }
    })
    .join()
    .map_err(|_| "System browser initialization failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_execute_receives_original_utf16_url() {
        let url = "HTTPS://example.org/測試?q=%E4%B8%AD#文";
        launch_with(WebUrl(url), |wide| {
            assert_eq!(wide.last(), Some(&0));
            assert_eq!(String::from_utf16(&wide[..wide.len() - 1]).unwrap(), url);
            Ok(33)
        })
        .unwrap();
    }

    #[test]
    fn shell_execute_status_and_errors_are_preserved() {
        for code in [0, 2, 31, 32] {
            assert!(launch_with(WebUrl("https://example.org"), |_| Ok(code)).is_err());
        }
        assert!(launch_with(WebUrl("https://example.org"), |_| Ok(33)).is_ok());
        assert_eq!(
            launch_with(WebUrl("https://example.org"), |_| Err("injected failure")),
            Err("injected failure")
        );
    }
}
