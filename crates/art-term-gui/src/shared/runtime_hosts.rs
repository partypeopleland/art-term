//! Native-only runtime Host management. No command returns a credential.

use art_term_core::{
    connections::Connections,
    openab::{
        api::{AdminClient, ApiError},
        profiles::{HostProfile, HostStore},
        secret::Secret,
    },
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

pub struct RuntimeHosts {
    pub store: Result<HostStore, ApiError>,
    pub admin: Result<AdminClient, ApiError>,
    /// Immutable non-secret defaults, used only when a Host has no local
    /// metadata. Matching requires the exact canonical origin.
    pub connections: Option<Connections>,
    #[cfg(all(debug_assertions, feature = "openab-fixture"))]
    pub fixture_ca: Result<Option<Vec<u8>>, ApiError>,
}
impl RuntimeHosts {
    pub async fn attach(
        &self,
        host: art_term_core::openab::profiles::HostSnapshot,
        grant: art_term_core::openab::api::SessionGrant,
        since: u64,
        generation: u64,
    ) -> Result<
        art_term_core::openab::session::SessionHandle,
        art_term_core::openab::session::SessionError,
    > {
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        if let Some(ca) = self
            .fixture_ca
            .as_ref()
            .map_err(|e| art_term_core::openab::session::SessionError::from(*e))?
        {
            return art_term_core::openab::session::SessionHandle::attach_fixture(
                host, grant, since, generation, ca,
            )
            .await;
        }
        art_term_core::openab::session::SessionHandle::attach(host, grant, since, generation).await
    }
    /// Attach by binding + retained token — same-token `?since=` resume
    /// and the dedicated screenshot Shell. No credential snapshot needed:
    /// the token is the only secret this path uses.
    pub async fn attach_token(
        &self,
        binding: art_term_core::openab::session::SessionBinding,
        token: art_term_core::openab::secret::Secret,
        since: u64,
        generation: u64,
    ) -> Result<
        art_term_core::openab::session::SessionHandle,
        art_term_core::openab::session::SessionError,
    > {
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        if let Some(ca) = self
            .fixture_ca
            .as_ref()
            .map_err(|e| art_term_core::openab::session::SessionError::from(*e))?
        {
            return art_term_core::openab::session::SessionHandle::attach_token_fixture(
                binding, token, since, generation, ca,
            )
            .await;
        }
        art_term_core::openab::session::SessionHandle::attach_token(binding, token, since, generation)
            .await
    }
    pub(crate) fn store(&self) -> Result<&HostStore, ApiError> {
        self.store.as_ref().map_err(|error| *error)
    }
    pub(crate) fn admin(&self) -> Result<&AdminClient, ApiError> {
        self.admin.as_ref().map_err(|error| *error)
    }
}

/// Deliberately no Debug/Serialize: this is a one-shot native input only.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostInput {
    id: Option<String>,
    expected_revision: Option<u64>,
    name: String,
    base_url: String,
    credential: String,
    metadata: Option<art_term_core::connections::ConnectionMetadata>,
    category: Option<art_term_core::connections::Category>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTest {
    session_count: usize,
    draining: bool,
    teardown_best_effort: bool,
    continuity_verified: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostError {
    kind: ApiError,
    message: &'static str,
}
impl From<ApiError> for HostError {
    fn from(kind: ApiError) -> Self {
        let message = match kind {
            ApiError::Auth => "主機認證被拒絕，請更新認證。",
            ApiError::Tls => "HTTPS 憑證驗證失敗，請確認網址與主機憑證。",
            ApiError::Transport => "無法連到主機，請確認網路與服務。",
            ApiError::Timeout => "主機回應逾時。",
            ApiError::GatewayUnavailable => "主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試。",
            ApiError::Throttled => "主機暫時限制請求，請稍後重試。",
            ApiError::Capacity => "主機或本機資料已達容量限制。",
            ApiError::SessionMissing => "遠端 Shell 已不存在。",
            ApiError::SessionDead => "遠端 Shell 已結束。",
            ApiError::UpgradeRequired => "主機 runtime 需要升級，才能使用此版本的 Shell 連線。",
            ApiError::Replacement => {
                "遠端 runtime 或 Shell 已更換，請重新選擇，原連線不會自動改綁。"
            }
            ApiError::Protocol => "資料格式不符；請使用完整 HTTPS 主機網址與有效認證。",
            ApiError::StateChanged => "主機資料已變更，請重新整理。",
            ApiError::Storage => "無法讀寫受保護的主機認證，原資料已保留。",
        };
        Self { kind, message }
    }
}
type Reply<T> = Result<T, HostError>;

#[cfg(all(debug_assertions, feature = "openab-fixture"))]
pub fn load_fixture_ca() -> Result<Option<Vec<u8>>, ApiError> {
    let Some(value) = std::env::var_os("ART_TERM_OPENAB_FIXTURE_CA") else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(value);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| ApiError::Storage)?;
    if !path.is_absolute()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 65536
    {
        return Err(ApiError::Storage);
    }
    Ok(Some(std::fs::read(path).map_err(|_| ApiError::Storage)?))
}

/// Safe saved-profile DTO with local metadata overriding exact-origin
/// catalog defaults. No credential is returned.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeHostDto {
    #[serde(flatten)]
    profile: HostProfile,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_tool: Option<String>,
}
impl RuntimeHostDto {
    fn resolved(profile: HostProfile, connections: Option<&Connections>) -> Self {
        let metadata = art_term_core::runtime_preview::resolve_metadata(
            profile.metadata.as_ref(),
            connections,
            &profile.base_url,
        );
        let agent = if metadata.category == art_term_core::connections::Category::Agent {
            connections.and_then(|c| c.connection(&profile.base_url))
        } else {
            None
        };
        Self {
            category: Some(
                if metadata.category == art_term_core::connections::Category::Agent {
                    "agent"
                } else {
                    "host"
                },
            ),
            agent_id: agent.map(|a| a.id.clone()),
            agent_name: None,
            agent_tool: metadata.coding_cli.as_ref().map(|c| c.name.clone()),
            profile,
        }
    }
}

#[tauri::command]
pub async fn gui_runtime_hosts(state: State<'_, Arc<RuntimeHosts>>) -> Reply<Vec<RuntimeHostDto>> {
    let hosts = state.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<Vec<RuntimeHostDto>, ApiError> {
        Ok(hosts
            .store()?
            .list()?
            .into_iter()
            .map(|profile| RuntimeHostDto::resolved(profile, hosts.connections.as_ref()))
            .collect())
    })
    .await
    .map_err(|_| HostError::from(ApiError::StateChanged))?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn gui_runtime_save_host(
    state: State<'_, Arc<RuntimeHosts>>,
    mut input: HostInput,
) -> Reply<RuntimeHostDto> {
    // Transfer the one-shot secret before validation, so every failure path
    // after deserialization drops a zeroizing Secret rather than a raw String.
    let value = std::mem::take(&mut input.credential);
    let credential = if value.is_empty() {
        None
    } else {
        Some(Secret::new(value).map_err(HostError::from)?)
    };
    let hosts = state.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<RuntimeHostDto, ApiError> {
        if input.category.is_some() && input.metadata.is_some() {
            return Err(ApiError::Protocol);
        }
        if let Some(category) = input.category {
            let profile = match (&input.id, input.expected_revision) {
                (Some(id), Some(revision)) => {
                    let p = hosts
                        .store()?
                        .list()?
                        .into_iter()
                        .find(|p| &p.id == id)
                        .ok_or(ApiError::StateChanged)?;
                    if p.revision != revision {
                        return Err(ApiError::StateChanged);
                    }
                    Some(p)
                }
                (None, None) => None,
                _ => return Err(ApiError::StateChanged),
            };
            input.metadata = Some(category_metadata(
                category,
                profile.as_ref(),
                hosts.connections.as_ref(),
                &input.base_url,
            )?);
        }
        let profile = match (input.id, input.expected_revision) {
            (Some(id), Some(revision)) => hosts.store()?.update_with_metadata(
                &id,
                revision,
                input.name,
                input.base_url,
                credential,
                input.metadata,
            )?,
            (None, None) => hosts.store()?.add_with_metadata(
                input.name,
                input.base_url,
                credential.ok_or(ApiError::Protocol)?,
                input.metadata,
            )?,
            _ => return Err(ApiError::StateChanged),
        };
        Ok(RuntimeHostDto::resolved(
            profile,
            hosts.connections.as_ref(),
        ))
    })
    .await
    .map_err(|_| HostError::from(ApiError::StateChanged))?
    .map_err(Into::into)
}

fn category_metadata(
    category: art_term_core::connections::Category,
    existing: Option<&HostProfile>,
    connections: Option<&Connections>,
    origin: &str,
) -> Result<art_term_core::connections::ConnectionMetadata, ApiError> {
    let origin = art_term_core::openab::profiles::canonical_origin(origin)?;
    let mut metadata = art_term_core::runtime_preview::resolve_metadata(
        existing.and_then(|p| p.metadata.as_ref()),
        connections,
        &origin,
    );
    metadata.category = category;
    Ok(metadata)
}

#[tauri::command]
pub async fn gui_runtime_delete_host(
    state: State<'_, Arc<RuntimeHosts>>,
    id: String,
    expected_revision: u64,
) -> Reply<()> {
    let hosts = state.inner().clone();
    tokio::task::spawn_blocking(move || hosts.store()?.delete(&id, expected_revision))
        .await
        .map_err(|_| HostError::from(ApiError::StateChanged))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn gui_runtime_test_host(
    state: State<'_, Arc<RuntimeHosts>>,
    id: String,
    expected_revision: u64,
) -> Reply<HostTest> {
    let hosts = state.inner().clone();
    let snapshot = hosts
        .store()
        .map_err(HostError::from)?
        .snapshot(&id)
        .map_err(HostError::from)?;
    if snapshot.profile.revision != expected_revision {
        return Err(ApiError::StateChanged.into());
    }
    let list = hosts
        .admin()
        .map_err(HostError::from)?
        .list(&snapshot)
        .await
        .map_err(HostError::from)?;
    if !hosts
        .store()
        .map_err(HostError::from)?
        .is_current(&snapshot.profile)
        .map_err(HostError::from)?
    {
        return Err(ApiError::StateChanged.into());
    }
    Ok(HostTest {
        session_count: list.sessions.len(),
        draining: list.draining,
        // Empty/unidentified runtime must not be represented as guaranteed.
        teardown_best_effort: list.sessions.is_empty()
            || list.sessions.iter().any(|s| s.teardown_best_effort),
        // Verified continuation needs the reviewed patched runtime and session
        // state machine; successful admin authentication alone is insufficient.
        continuity_verified: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_failure_serializes_a_fixed_local_message() {
        let value = serde_json::to_value(HostError::from(ApiError::GatewayUnavailable)).unwrap();
        assert_eq!(value["kind"], "gatewayUnavailable");
        assert_eq!(
            value["message"],
            "主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試。"
        );
    }
    #[test]
    fn category_update_preserves_local_and_exact_origin_catalog_capabilities() {
        use art_term_core::connections::{Category, CodingCli, Connection, ConnectionMetadata};
        let capabilities = ConnectionMetadata {
            category: Category::Agent,
            desktop_transport: Some("runtime-mcp".into()),
            coding_cli: Some(CodingCli {
                id: "assistant".into(),
                name: "Assistant".into(),
                command: "assistant-cli".into(),
                args: vec![],
            }),
        };
        let catalog = Connections {
            generation: "fixture".into(),
            connections: vec![Connection {
                id: "fixture".into(),
                name: "Fixture".into(),
                openab_pty_url: "https://fixture.test/".into(),
                metadata: capabilities.clone(),
            }],
            migration_warnings: vec![],
        };
        let mut profile = HostProfile {
            id: "fixture".into(),
            revision: 7,
            name: "Fixture".into(),
            base_url: "https://fixture.test/".into(),
            credential_present: true,
            metadata: None,
        };
        for existing in [None, Some(&profile)] {
            let changed = category_metadata(
                Category::Host,
                existing,
                Some(&catalog),
                "https://fixture.test:443/",
            )
            .unwrap();
            assert_eq!(changed.category, Category::Host);
            assert_eq!(changed.desktop_transport, capabilities.desktop_transport);
            assert_eq!(changed.coding_cli, capabilities.coding_cli);
        }
        let moved = category_metadata(
            Category::Agent,
            Some(&profile),
            Some(&catalog),
            "https://other.test",
        )
        .unwrap();
        assert!(moved.desktop_transport.is_none());
        assert!(moved.coding_cli.is_none());
        profile.metadata = Some(capabilities.clone());
        let local = category_metadata(
            Category::Host,
            Some(&profile),
            Some(&catalog),
            "https://other.test",
        )
        .unwrap();
        assert_eq!(local.desktop_transport, capabilities.desktop_transport);
        assert_eq!(local.coding_cli, capabilities.coding_cli);
        let input: HostInput = serde_json::from_value(serde_json::json!({ "name":"Fixture", "baseUrl":"https://fixture.test", "credential":"fixture", "category":"agent" })).unwrap();
        assert_eq!(input.category, Some(Category::Agent));
    }
    #[test]
    fn metadata_ipc_and_dto_work_without_catalog_or_private_build() {
        let input: HostInput = serde_json::from_value(serde_json::json!({
            "name": "Local name", "baseUrl": "https://local.test/", "credential": "fixture",
            "metadata": {"category":"agent", "desktop_transport":"runtime-mcp", "coding_cli": {"id":"assistant", "name":"Assistant", "command":"assistant-cli", "args":["--model", "custom"]}}
        })).unwrap();
        let profile = HostProfile {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 7,
            name: input.name,
            base_url: input.base_url,
            credential_present: true,
            metadata: input.metadata,
        };
        let dto = serde_json::to_value(RuntimeHostDto::resolved(profile.clone(), None)).unwrap();
        assert_eq!(dto["category"], "agent");
        assert_eq!(dto["name"], "Local name");
        assert_eq!(dto["revision"], 7);
        assert!(dto.get("credential").is_none());
        let metadata = art_term_core::runtime_preview::resolve_metadata(
            profile.metadata.as_ref(),
            None,
            &profile.base_url,
        );
        let mapping = art_term_core::runtime_preview::mapping_from_metadata(&metadata);
        assert!(mapping.route.is_some());
        assert_eq!(mapping.coding_cli.unwrap().command, "assistant-cli");
    }
}
