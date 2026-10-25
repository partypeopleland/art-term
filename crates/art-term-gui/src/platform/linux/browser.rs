use super::WebUrl;

pub(super) fn open(url: WebUrl<'_>) -> Result<(), &'static str> {
    gio::AppInfo::launch_default_for_uri(url.0, None::<&gio::AppLaunchContext>)
        .map_err(|_| "Unable to open system browser")
}
