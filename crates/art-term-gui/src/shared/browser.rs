//! Validate web URLs once before handing them to the native desktop association.
#[cfg(windows)]
#[path = "../platform/windows/browser.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "../platform/linux/browser.rs"]
mod native;

struct WebUrl<'a>(&'a str);

fn validate(url: &str) -> Result<WebUrl<'_>, &'static str> {
    if url.is_empty()
        || url.len() > 8192
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("Invalid web URL");
    }
    let (scheme, rest) = url.split_once("://").ok_or("Invalid web URL")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || rest.split(['/', '?', '#']).next().unwrap_or("").is_empty()
    {
        return Err("Invalid web URL");
    }
    let parsed = url::Url::parse(url).map_err(|_| "Invalid web URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
        return Err("Invalid web URL");
    }
    Ok(WebUrl(url))
}
fn launch_with(
    url: &str,
    opener: impl FnOnce(WebUrl<'_>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    opener(validate(url)?)
}

pub(crate) fn open_web_url(url: &str) -> Result<(), &'static str> {
    launch_with(url, native::open)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_boundary_rejects_unsafe_input_without_calling_launcher() {
        for url in [
            "",
            "javascript:alert(1)",
            "file:///C:/a",
            "https://",
            "https:///example.org",
            "https:example.org",
            "https://e.test/\nnext",
            "https://e.test/\0",
            "https://e.test/a b",
            "https://e.test/\u{7f}",
        ] {
            assert!(
                launch_with(url, |_| panic!("invalid URL reached launcher")).is_err(),
                "{url:?}"
            );
        }
        let long = format!("https://e.test/{}", "x".repeat(8192));
        assert!(launch_with(&long, |_| panic!("oversize reached launcher")).is_err());
    }
    #[test]
    fn browser_boundary_preserves_original_url_and_opens_exactly_once() {
        for url in [
            "http://example.org/a?x=1&y=2#fragment",
            "HTTPS://example.org/測試?q=%E4%B8%AD#文",
        ] {
            let calls = std::cell::Cell::new(0);
            launch_with(url, |wide| {
                calls.set(calls.get() + 1);
                assert_eq!(wide.0, url);
                Ok(())
            })
            .unwrap();
            assert_eq!(calls.get(), 1);
        }
    }
    #[test]
    fn browser_boundary_preserves_native_launch_failure() {
        assert_eq!(
            launch_with("https://example.org", |_| Err("injected launch failure")),
            Err("injected launch failure")
        );
    }
}
