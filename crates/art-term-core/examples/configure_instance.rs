//! Deterministic native-path driver, enabled only for local fixture builds.
#[cfg(all(debug_assertions, feature = "openab-fixture"))]
#[tokio::main]
async fn main() {
    use art_term_core::openab::{
        api::AdminClient,
        profiles::{HostProfile, HostSnapshot},
        screenshot::{self, ShellOwnership},
        secret::Secret,
        session::SessionHandle,
    };
    use std::{
        io::Read,
        sync::{Arc, Mutex},
    };
    struct Owned(Mutex<Vec<String>>);
    impl ShellOwnership for Owned {
        fn mark(&self, n: &str) -> Result<(), art_term_core::openab::session::SessionError> {
            self.0.lock().unwrap().push(n.into());
            Ok(())
        }
        fn unmark(&self, _: &str) {}
    }
    let result = async {
        let mut raw = zeroize::Zeroizing::new(String::new());
        std::io::stdin().read_to_string(&mut raw)?;
        let v: serde_json::Value = serde_json::from_str(&raw)?;
        let ca = Arc::new(std::fs::read(v["ca"].as_str().unwrap())?);
        let api = AdminClient::fixture_ca(&ca)?;
        let host = HostSnapshot {
            profile: HostProfile {
                id: "fixture-host".into(),
                revision: 1,
                name: "Fixture".into(),
                base_url: v["origin"].as_str().unwrap().into(),
                credential_present: true,
                metadata: None,
            },
            credential: Secret::new(v["credential"].as_str().unwrap().into())?,
        };
        let owned = Arc::new(Owned(Mutex::new(vec![])));
        let (_cancel, cancel) = tokio::sync::watch::channel(0);
        let reply = screenshot::configure(
            &api,
            host,
            v["binding"].as_str().unwrap(),
            zeroize::Zeroizing::new(v["payload"].to_string()),
            owned.clone(),
            cancel,
            move |b, t, s, g| {
                let ca = ca.clone();
                async move { SessionHandle::attach_token_fixture(b, t, s, g, &ca).await }
            },
        )
        .await?;
        let mut out = serde_json::json!({"result":reply,"owned":owned.0.lock().unwrap().clone()});
        if v["payload"]["op"] == "configure" {
            let p = &v["payload"];
            let shot = art_term_core::instance_mcp::screenshot(
                art_term_core::instance_mcp::DesktopSnapshot {
                    profile: art_term_core::instance_mcp::DesktopProfile {
                        id: p["id"].as_str().unwrap().into(),
                        revision: 1,
                        name: p["name"].as_str().unwrap().into(),
                        mcp_url: p["url"].as_str().unwrap().into(),
                        token_present: true,
                    },
                    token: Secret::new(p["token"].as_str().unwrap().into())?,
                },
            )
            .await?;
            out["image"] = serde_json::Value::String(shot.image_base64);
        }
        Ok::<_, anyhow::Error>(out)
    }
    .await;
    match result {
        Ok(v) => println!("{v}"),
        Err(_) => {
            eprintln!("native fixture operation failed");
            std::process::exit(1);
        }
    }
}
#[cfg(not(all(debug_assertions, feature = "openab-fixture")))]
fn main() {
    panic!("fixture-only debug driver");
}
