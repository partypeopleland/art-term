use tauri::{plugin::TauriPlugin, Runtime};

#[cfg(target_os = "android")]
pub struct Vault<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> Vault<R> {
    /// Native-only operator vault initialization. No WebView ACL is granted.
    pub fn operator_root(&self) -> Result<std::path::PathBuf, String> {
        #[derive(serde::Deserialize)]
        struct Reply {
            path: String,
        }
        let reply: Reply = self
            .0
            .run_mobile_plugin("operatorInitialize", ())
            .map_err(|_| "Cannot initialize operator vault".to_owned())?;
        let path = std::path::PathBuf::from(reply.path);
        if !path.is_absolute() {
            return Err("Cannot initialize operator vault".to_owned());
        }
        Ok(path)
    }

    pub fn operator_seal(&self, context: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
        #[derive(serde::Serialize)]
        struct Args<'a> {
            context: &'a [u8],
            data: &'a [u8],
        }
        #[derive(serde::Deserialize)]
        struct Reply {
            data: Vec<u8>,
        }
        let reply: Reply = self
            .0
            .run_mobile_plugin("operatorSeal", Args { context, data })
            .map_err(|_| "Cannot save operator credential".to_owned())?;
        Ok(reply.data)
    }

    pub fn operator_unseal(
        &self,
        context: &[u8],
        data: &[u8],
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
        #[derive(serde::Serialize)]
        struct Args<'a> {
            context: &'a [u8],
            data: &'a [u8],
        }
        #[derive(serde::Deserialize)]
        struct Reply {
            data: Vec<u8>,
        }
        let reply: Reply = self
            .0
            .run_mobile_plugin("operatorUnseal", Args { context, data })
            .map_err(|_| "Cannot unlock operator credential".to_owned())?;
        Ok(zeroize::Zeroizing::new(reply.data))
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("mobile-vault")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    _api.register_android_plugin("net.missart.artterm.vault", "MobileVaultPlugin")?;
                _app.manage(Vault(handle));
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    #[test]
    fn operator_vault_commands_have_no_webview_grant() {
        let capability = include_str!("../../art-term-gui/capabilities/main.json");
        assert!(!capability.contains("mobile-vault"));
        assert!(!capability.contains("sshPrepare"));
        assert!(!capability.contains("sshCommit"));
    }
}
