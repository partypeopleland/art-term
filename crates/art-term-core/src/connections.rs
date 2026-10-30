//! Bounded runtime catalog with migration-only legacy endpoint inputs.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Whole-file byte bound from the schema contract.
pub const SOURCE_MAX_BYTES: usize = 1024 * 1024;
const ENDPOINTS_MAX: usize = 32;
const RUNTIME_AGENTS_MAX: usize = 64;
const HOST_KEY_LINE_MAX: usize = 1024;
/// Target id charset/length — same rule the retired catalog enforced.
const ID_MAX: usize = 32;

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && b == b'-'))
}

fn valid_display(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() <= max && !s.chars().any(char::is_control)
}

/// Non-secret metadata shared by catalog entries and persisted Hosts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodingCli {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}
impl CodingCli {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            valid_id(&self.id) && valid_display(&self.name, 80),
            "invalid coding CLI identity"
        );
        ensure!(
            !self.command.is_empty()
                && self.command.len() <= 256
                && !self.command.starts_with('-')
                && !self.command.chars().any(char::is_control),
            "invalid coding CLI command"
        );
        ensure!(
            self.args.len() <= 32
                && self
                    .args
                    .iter()
                    .all(|a| a.len() <= 1024 && !a.chars().any(char::is_control)),
            "invalid coding CLI args"
        );
        ensure!(
            self.command.len() + self.args.iter().map(String::len).sum::<usize>() <= 8192,
            "coding CLI exceeds bounds"
        );
        Ok(())
    }
}
fn legacy_cli(value: Option<String>) -> Option<CodingCli> {
    value.map(|_| CodingCli {
        id: "codex".into(),
        name: "Codex".into(),
        command: "codex".into(),
        args: vec!["--yolo".into()],
    })
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Host,
    Agent,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub category: Category,
    #[serde(default)]
    pub desktop_transport: Option<String>,
    #[serde(default)]
    pub coding_cli: Option<CodingCli>,
}
impl Default for ConnectionMetadata {
    fn default() -> Self {
        Self {
            display_name: None,
            category: Category::Host,
            desktop_transport: None,
            coding_cli: None,
        }
    }
}
impl ConnectionMetadata {
    pub fn normalize_display_name(&mut self) {
        self.display_name = self
            .display_name
            .take()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(name) = &self.display_name {
            ensure!(
                name.chars().count() <= 20 && !name.chars().any(char::is_control),
                "invalid display name"
            );
        }
        ensure!(
            self.desktop_transport
                .as_deref()
                .is_none_or(|v| v == "runtime-mcp"),
            "unsupported desktop transport"
        );
        if let Some(cli) = &self.coding_cli {
            cli.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub openab_pty_url: String,
    pub metadata: ConnectionMetadata,
}
#[derive(Clone, Debug)]
pub struct Connections {
    pub generation: String,
    pub connections: Vec<Connection>,
    pub migration_warnings: Vec<String>,
}
impl Connections {
    pub fn connection(&self, canonical_origin: &str) -> Option<&Connection> {
        self.connections
            .iter()
            .find(|c| c.openab_pty_url == canonical_origin)
    }
}

// ---------- serde input shapes (deny_unknown_fields at every level) ----------

/// Top-level shape covering every field both accepted schema versions
/// ever defined. Retired `computers`/`agents` arrays deserialize as raw
/// values only so a schema v2 file can be *checked for emptiness* — the
/// entries inside are never interpreted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    schema_version: u32,
    connections: Option<Vec<RawConnection>>,
    auto_discover: Option<bool>,
    endpoints: Option<Vec<RawEndpoint>>,
    computers: Option<Vec<toml::Value>>,
    agents: Option<Vec<toml::Value>>,
    runtime_agents: Option<Vec<RawRuntimeAgent>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConnection {
    display_name: Option<String>,
    id: String,
    name: String,
    openab_pty_url: String,
    category: Category,
    desktop_transport: Option<String>,
    coding_cli: Option<CodingCli>,
}
// Migration-only shape; never used by runtime routing.
struct Endpoint {
    id: String,
    runtime_origin: Option<String>,
    coding_cli: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndpoint {
    id: String,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    network: Option<String>,
    expected_host_key_algorithm: Option<String>,
    expected_host_key: Option<String>,
    expected_host_key_sha256: Option<String>,
    /// Compatibility input only — bounded, never used to compose a
    /// terminal command in this build.
    tmux_conf: Option<String>,
    desktop_provider: Option<String>,
    /// Optional exact canonical HTTPS origin of a runtime Host
    /// (openab-pty) that shares this endpoint's desktop provider —
    /// This exact origin is retained; all SSH fields are discarded.
    runtime_origin: Option<String>,
    /// Optional coding CLI this endpoint's mapped targets advertise
    /// (`"codex"` is the only supported value).
    coding_cli: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRuntimeAgent {
    id: String,
    name: String,
    tool: String,
    /// Exact canonical HTTPS origin of the runtime Host. Non-secret
    /// binding metadata — credentials never belong in this file.
    origin: String,
    /// Optional id of a `desktop_provider = "art-term-hands"` endpoint.
    desktop_endpoint: Option<String>,
    /// Optional desktop transport — `"runtime-mcp"` only; mutually
    /// exclusive with `desktop_endpoint`.
    desktop_transport: Option<String>,
    /// Optional coding CLI hint (`"codex"` only).
    coding_cli: Option<String>,
}

/// 1-based (line, column) of `offset` in `source` — for safe error
/// positions that never echo source text.
fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for b in source.bytes().take(offset.min(source.len())) {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Parse and fully validate a connections source. Returns a clear error
/// for every contract violation — callers must surface it, never fall
/// back to any built-in catalog.
pub fn parse(source: &str) -> Result<Connections> {
    ensure!(
        source.len() <= SOURCE_MAX_BYTES,
        "connections file exceeds 1 MiB"
    );
    let raw: RawFile = toml::from_str(source).map_err(|e| {
        // The rendered TOML error echoes the offending source line —
        // Even the deserializer's message can echo a pasted value or
        // unknown field name. Only a fixed reason and position may
        // cross the IPC boundary.
        let (line, col) = e
            .span()
            .map(|s| line_col(source, s.start))
            .unwrap_or((0, 0));
        anyhow::anyhow!("invalid TOML or configuration fields at line {line}, column {col}")
    })?;
    match raw.schema_version {
        4 => parse_v4(raw, source),
        3 => parse_v3(raw, source),
        2 => parse_v2(raw, source),
        _ => bail!("unsupported schema_version; expected 2, 3 or 4"),
    }
}

fn generation(source: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(source.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn parse_v4(raw: RawFile, source: &str) -> Result<Connections> {
    ensure!(
        raw.auto_discover.is_none()
            && raw.endpoints.is_none()
            && raw.computers.is_none()
            && raw.agents.is_none()
            && raw.runtime_agents.is_none(),
        "legacy fields are not part of schema_version 4"
    );
    let items = raw
        .connections
        .context("connections is required for schema_version 4")?;
    ensure!(items.len() <= 64, "too many connections (max 64)");
    let mut ids = HashSet::new();
    let mut origins = HashSet::new();
    let mut connections = Vec::new();
    for c in items {
        ensure!(
            valid_id(&c.id) && ids.insert(c.id.clone()),
            "invalid or duplicate connection id"
        );
        ensure!(valid_display(&c.name, 80), "invalid connection name");
        let origin = crate::openab::profiles::canonical_origin(&c.openab_pty_url)
            .map_err(|_| anyhow::anyhow!("invalid openab_pty_url"))?;
        ensure!(
            origins.insert(origin.clone()),
            "duplicate connection origin"
        );
        let metadata = ConnectionMetadata {
            display_name: c.display_name,
            category: c.category,
            desktop_transport: c.desktop_transport,
            coding_cli: c.coding_cli,
        };
        metadata.validate()?;
        connections.push(Connection {
            id: c.id,
            name: c.name.trim().into(),
            openab_pty_url: origin,
            metadata,
        });
    }
    Ok(Connections {
        generation: generation(source),
        connections,
        migration_warnings: Vec::new(),
    })
}

/// Schema v3: `schema_version` + `runtime_agents`, with `endpoints`
/// allowed as the desktop-provider catalog. Any other retired field —
/// even empty — is a clear error, never silently ignored.
fn parse_v3(raw: RawFile, source: &str) -> Result<Connections> {
    ensure!(
        raw.connections.is_none() && raw.auto_discover.is_none() && raw.computers.is_none() && raw.agents.is_none(),
        "retired fields are not part of schema_version 3; keep only schema_version, runtime_agents and optional endpoints"
    );
    let agents = raw
        .runtime_agents
        .context("runtime_agents is required for schema_version 3")?;
    finish(raw.endpoints.unwrap_or_default(), agents, source)
}

/// Schema v2: accepted when `auto_discover = false`, the retired
/// `computers`/`agents` arrays are present and empty, and `endpoints`
/// is present — the desktop-provider catalog is parsed and validated.
/// Any non-empty retired array is a real binding this build cannot
/// honor — reject it explicitly instead of discarding it silently.
fn parse_v2(raw: RawFile, source: &str) -> Result<Connections> {
    ensure!(
        raw.connections.is_none() && raw.auto_discover == Some(false),
        "unsupported schema_version 2 file; auto_discover must be false"
    );
    ensure!(
        raw.endpoints.is_some()
            && raw.computers.is_some_and(|v| v.is_empty())
            && raw.agents.is_some_and(|v| v.is_empty()),
        "schema_version 2 file must declare endpoints with empty computers/agents; migrate retired arrays to schema_version 3"
    );
    finish(
        raw.endpoints.unwrap_or_default(),
        raw.runtime_agents.unwrap_or_default(),
        source,
    )
}

fn finish(
    raw_endpoints: Vec<RawEndpoint>,
    raw_agents: Vec<RawRuntimeAgent>,
    source: &str,
) -> Result<Connections> {
    ensure!(
        raw_endpoints.len() <= ENDPOINTS_MAX,
        "too many endpoints (max {ENDPOINTS_MAX})"
    );
    ensure!(
        raw_agents.len() <= RUNTIME_AGENTS_MAX,
        "too many runtime_agents (max {RUNTIME_AGENTS_MAX})"
    );

    let mut endpoint_ids = HashSet::new();
    let mut runtime_origins = HashSet::new();
    let mut endpoints = Vec::with_capacity(raw_endpoints.len());
    for ep in raw_endpoints {
        ensure!(
            valid_id(&ep.id) && endpoint_ids.insert(ep.id.clone()),
            "invalid or duplicate endpoint id"
        );
        // Legacy SSH fields are bounded migration inputs only. Do not decode keys,
        // check pins, or retain anything capable of constructing an executor.
        for value in [
            &ep.host,
            &ep.username,
            &ep.network,
            &ep.expected_host_key_algorithm,
            &ep.expected_host_key,
            &ep.expected_host_key_sha256,
            &ep.tmux_conf,
        ] {
            if let Some(value) = value {
                ensure!(
                    value.len() <= HOST_KEY_LINE_MAX && !value.chars().any(char::is_control),
                    "legacy endpoint field exceeds bounds"
                );
            }
        }
        let _ = ep.port;
        if let Some(provider) = &ep.desktop_provider {
            ensure!(provider == "art-term-hands", "unsupported desktop_provider");
        }
        if let Some(cli) = &ep.coding_cli {
            ensure!(cli == "codex", "unsupported endpoint coding_cli");
        }
        let runtime_origin = ep
            .runtime_origin
            .as_ref()
            .map(|origin| {
                crate::openab::profiles::canonical_origin(origin)
                    .map_err(|_| anyhow::anyhow!("invalid endpoint runtime_origin"))
            })
            .transpose()?;
        if let Some(origin) = &runtime_origin {
            ensure!(
                runtime_origins.insert(origin.clone()),
                "duplicate endpoint runtime_origin"
            );
        }
        endpoints.push(Endpoint {
            id: ep.id,
            runtime_origin,
            coding_cli: ep.coding_cli,
        });
    }

    // Declared runtime agents: ids are unique, and origins are unique
    // among themselves and never equal an endpoint's `runtime_origin` —
    // one origin, one binding model (`runtime_origins` was seeded by
    // the endpoint loop above). A `desktop_endpoint` must name an
    // existing legacy endpoint without a conflicting origin and
    // is mutually exclusive with `desktop_transport` (one origin, one
    // frozen route).
    let raw_endpoint_links: HashSet<String> = raw_agents
        .iter()
        .filter_map(|a| a.desktop_endpoint.clone())
        .collect();
    let mut runtime_agent_ids = HashSet::new();
    let mut runtime_agents = Vec::with_capacity(raw_agents.len());
    for a in raw_agents {
        ensure!(
            valid_id(&a.id),
            "invalid runtime agent id (1-32 lowercase ascii/digit/hyphen)"
        );
        ensure!(
            runtime_agent_ids.insert(a.id.clone()),
            "duplicate runtime agent id"
        );
        ensure!(valid_display(&a.name, 80), "invalid runtime agent name");
        ensure!(valid_display(&a.tool, 32), "invalid runtime agent tool");
        let origin = crate::openab::profiles::canonical_origin(&a.origin)
            .map_err(|_| anyhow::anyhow!("invalid runtime agent origin"))?;
        ensure!(
            runtime_origins.insert(origin.clone()),
            "runtime agent origin duplicates another binding"
        );
        let desktop_endpoint = match a.desktop_endpoint.as_deref() {
            Some(id) => {
                let endpoint = endpoints
                    .iter()
                    .find(|e| e.id == id)
                    .context("runtime agent desktop_endpoint does not exist")?;
                ensure!(
                    endpoint
                        .runtime_origin
                        .as_deref()
                        .is_none_or(|endpoint_origin| endpoint_origin == origin),
                    "runtime agent desktop_endpoint belongs to another runtime origin"
                );
                Some(endpoint.id.clone())
            }
            None => None,
        };
        // `runtime-mcp` selects the runtime Host's own authenticated
        // desktop admin API — never an SSH route, and it can never be
        // combined with a legacy `desktop_endpoint` (one origin, one
        // frozen route). Any other value is a hard parse error.
        let desktop_transport = match a.desktop_transport.as_deref() {
            Some("runtime-mcp") => {
                ensure!(
                    desktop_endpoint.is_none(),
                    "runtime agent desktop_transport is mutually exclusive with desktop_endpoint"
                );
                Some("runtime-mcp".to_owned())
            }
            Some(_) => bail!("unsupported runtime agent desktop_transport"),
            None => None,
        };
        if let Some(cli) = &a.coding_cli {
            ensure!(cli == "codex", "unsupported runtime agent coding_cli");
        }
        runtime_agents.push(Connection {
            id: a.id,
            name: a.name.trim().to_owned(),
            openab_pty_url: origin,
            metadata: ConnectionMetadata {
                display_name: None,
                category: Category::Agent,
                desktop_transport: desktop_transport
                    .or_else(|| desktop_endpoint.map(|_| "runtime-mcp".to_owned())),
                coding_cli: legacy_cli(a.coding_cli),
            },
        });
    }

    let migration_warnings = endpoints.iter().filter(|ep| ep.runtime_origin.is_none() && !raw_endpoint_links.contains(&ep.id))
        .map(|ep| format!("舊 SSH endpoint {} 未宣告 Runtime origin；桌面不可用，請設定 Runtime HTTPS origin。", ep.id)).collect();
    let mut connections: Vec<Connection> = endpoints
        .into_iter()
        .filter_map(|ep| {
            ep.runtime_origin.map(|origin| Connection {
                name: ep.id.clone(),
                id: ep.id,
                openab_pty_url: origin,
                metadata: ConnectionMetadata {
                    display_name: None,
                    category: Category::Host,
                    desktop_transport: Some("runtime-mcp".into()),
                    coding_cli: legacy_cli(ep.coding_cli),
                },
            })
        })
        .collect();
    connections.extend(runtime_agents);
    Ok(Connections {
        generation: generation(source),
        connections,
        migration_warnings,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const UNIFIED: &str = r#"
schema_version = 4
[[connections]]
id = "workstation"
name = "My workstation"
openab_pty_url = "https://EXAMPLE.test:443/"
category = "host"
desktop_transport = "runtime-mcp"
coding_cli = { id = "assistant", name = "Assistant", command = "assistant", args = ["--model", "some model", "owner's workspace", "$(literal)"] }
[[connections]]
id = "agent"
name = "Agent"
openab_pty_url = "https://agent.test/"
category = "agent"
coding_cli = { id = "another", name = "Another CLI", command = "another-cli", args = [] }
"#;
    #[test]
    fn unified_schema_capabilities_are_independent_of_category() {
        let c = parse(UNIFIED).unwrap();
        assert_eq!(c.connections.len(), 2);
        assert_eq!(c.connections[0].openab_pty_url, "https://example.test/");
        assert_eq!(c.connections[0].metadata.category, Category::Host);
        assert_eq!(
            c.connections[0].metadata.coding_cli.as_ref().unwrap().args[2],
            "owner's workspace"
        );
        assert!(c.connections[1].metadata.desktop_transport.is_none());
        assert_eq!(
            c.connections[1]
                .metadata
                .coding_cli
                .as_ref()
                .unwrap()
                .command,
            "another-cli"
        );
    }
    #[test]
    fn unified_schema_rejects_unknown_secrets_duplicates_and_unbounded_cli() {
        for source in [
            UNIFIED.replace("https://agent.test/", "https://example.test/"),
            UNIFIED.replace(r#"id = "agent""#, r#"id = "workstation""#),
            UNIFIED.replace(r#"category = "host""#, r#"category = "computer""#),
            UNIFIED.replace(r#"command = "assistant""#, r#"command = "bad\ncommand""#),
            UNIFIED.replace("args = []", r#"args = ["bad\rarg"]"#),
            UNIFIED.replace(
                "args = []",
                &format!("args = [{}]", vec![r#""x""#; 33].join(",")),
            ),
            UNIFIED.replace(
                r#"name = "Agent""#,
                &format!(r#"name = "{}""#, "x".repeat(81)),
            ),
            format!("{UNIFIED}\npassword = \"not-public\""),
            format!("schema_version = 4\nruntime_agents = []\nconnections = []"),
            "schema_version = 4".into(),
        ] {
            assert!(parse(&source).is_err(), "accepted invalid schema4");
        }
        assert!(parse("schema_version = 4\nconnections = []").is_ok());
        let mut many = "schema_version = 4\n".to_owned();
        for i in 0..65 {
            many.push_str(&format!("[[connections]]\nid=\"c{i}\"\nname=\"n\"\ncategory=\"host\"\nopenab_pty_url=\"https://c{i}.test/\"\n"));
        }
        assert!(parse(&many).is_err());
    }

    /// The reviewed schema v3 contract — runtime agents only.
    pub(crate) const CONTRACT: &str = r##"
schema_version = 3

[[runtime_agents]]
id = "nuc-agent"
name = "NUC agent"
tool = "codex"
origin = "https://runtime.nuc.example.invalid/"
desktop_transport = "runtime-mcp"
coding_cli = "codex"

[[runtime_agents]]
id = "hana-agent"
name = "Hana"
tool = "shell"
origin = "https://runtime.hana.example.invalid/"
"##;

    /// Valid endpoint table body for both schema versions.
    fn endpoint_table(id: &str, extra: &str) -> String {
        let key = "legacy-public-key";
        let fp = "legacy-fingerprint";
        format!(
            "[[endpoints]]\nid = \"{id}\"\nhost = \"nuc.example.invalid\"\nport = 22\nusername = \"alice\"\nnetwork = \"tailnet\"\nexpected_host_key_algorithm = \"ssh-ed25519\"\nexpected_host_key = \"{key}\"\nexpected_host_key_sha256 = \"{fp}\"\n{extra}"
        )
    }

    /// Reviewed endpoint fixture: a `art-term-hands` provider endpoint
    /// bound to an exact canonical `runtime_origin`.
    pub(crate) const ENDPOINT_CONTRACT: &str = r##"
schema_version = 3
runtime_agents = []

[[endpoints]]
id = "nuc"
host = "nuc.example.invalid"
port = 22
username = "alice"
network = "tailnet"
expected_host_key_algorithm = "ssh-ed25519"
expected_host_key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKurq6urq6urq6urq6urq6urq6urq6urq6urq6urq6ur"
expected_host_key_sha256 = "SHA256:mEwqXdTPoHnFTvEV3e8U+5RuFC2tSO9zs8CTJPIXq/w"
tmux_conf = "/home/alice/.tmux.conf"
desktop_provider = "art-term-hands"
runtime_origin = "https://runtime.nuc.example.invalid/"
coding_cli = "codex"
"##;

    #[test]
    fn native_and_legacy_migrations_keep_origin_metadata_and_deny_guesses() {
        let parsed = parse(ENDPOINT_CONTRACT).unwrap();
        assert_eq!(
            Some(parsed.connections[0].openab_pty_url.as_str()),
            Some("https://runtime.nuc.example.invalid/")
        );
        assert_eq!(
            parsed.connections[0]
                .metadata
                .coding_cli
                .as_ref()
                .map(|c| c.id.as_str()),
            Some("codex")
        );
        assert_eq!(parsed.connections[0].metadata.category, Category::Host); // Computer classification stays unchanged.
        assert!(parsed.migration_warnings.is_empty());
        let source = format!("{}\n[[runtime_agents]]\nid = \"desk\"\nname = \"Desk\"\ntool = \"codex\"\norigin = \"https://other.example.invalid/\"\ndesktop_endpoint = \"nuc\"\ncoding_cli = \"codex\"\n", ENDPOINT_CONTRACT.replace("runtime_agents = []\n", "").replace("runtime_origin = \"https://runtime.nuc.example.invalid/\"\n", ""));
        let migrated = parse(&source).unwrap();
        assert_eq!(
            migrated.connections[0]
                .metadata
                .desktop_transport
                .as_deref(),
            Some("runtime-mcp")
        );
        assert_eq!(
            migrated.connections[0]
                .metadata
                .coding_cli
                .as_ref()
                .map(|c| c.id.as_str()),
            Some("codex")
        );
        assert!(migrated.migration_warnings.is_empty());
        assert!(parse(&source.replace(
            "https://other.example.invalid/",
            "http://other.example.invalid/"
        ))
        .is_err());
    }
    #[test]
    fn ssh_only_endpoint_warns_without_rejecting_valid_runtime() {
        let source = format!("{}\n{}", CONTRACT, endpoint_table("orphan", ""));
        let parsed = parse(&source).unwrap();
        assert_eq!(parsed.connections.len(), 2);
        assert_eq!(parsed.connections.len(), 2);
        assert!(parsed.migration_warnings[0].contains("桌面不可用"));
    }
    #[test]
    fn secrets_bounds_and_generation_still_enforced() {
        assert!(parse(&format!("{}\npassword = \"secret\"", CONTRACT)).is_err());
        assert!(parse(&ENDPOINT_CONTRACT.replace("alice", &"a".repeat(1025))).is_err());
        assert_ne!(
            parse(CONTRACT).unwrap().generation,
            parse(&CONTRACT.replace("NUC agent", "Other"))
                .unwrap()
                .generation
        );
        let err = parse("schema_version = 3\ntoken = \"must-not-leak\"\nruntime_agents = []")
            .unwrap_err()
            .to_string();
        assert!(!err.contains("must-not-leak"));
    }
    #[test]
    fn schema_v3_parses_runtime_agents() {
        let parsed = parse(CONTRACT).unwrap();
        assert_eq!(parsed.connections.len(), 2);
        let nuc = parsed.connections[0].clone();
        assert_eq!(nuc.id, "nuc-agent");
        assert_eq!(nuc.openab_pty_url, "https://runtime.nuc.example.invalid/");
        assert_eq!(
            nuc.metadata.desktop_transport.as_deref(),
            Some("runtime-mcp")
        );
        assert_eq!(
            nuc.metadata.coding_cli.as_ref().map(|c| c.id.as_str()),
            Some("codex")
        );
        assert!(parsed
            .connection("https://runtime.hana.example.invalid/")
            .is_some());
        assert!(parsed
            .connection("https://other.example.invalid/")
            .is_none());
        assert_eq!(parsed.generation.len(), 16);
    }
    #[test]
    fn generation_is_content_derived() {
        let a = parse(CONTRACT).unwrap();
        let b = parse(CONTRACT).unwrap();
        assert_eq!(a.generation, b.generation);
        let edited = parse(&CONTRACT.replace("nuc-agent", "nuc-agent2")).unwrap();
        assert_ne!(a.generation, edited.generation);
    }
    #[test]
    fn schema_v2_with_endpoints_and_empty_retired_arrays_parses() {
        let source = format!(
            "schema_version = 2\nauto_discover = false\ncomputers = []\nagents = []\n{}",
            endpoint_table("nuc", "desktop_provider = \"art-term-hands\"\nruntime_origin = \"https://runtime.nuc.example.invalid/\"")
        );
        let parsed = parse(&source).unwrap();
        assert_eq!(parsed.connections.len(), 1);
        assert_eq!(
            Some(parsed.connections[0].openab_pty_url.as_str()),
            Some("https://runtime.nuc.example.invalid/")
        );
    }
    #[test]
    fn schema_v2_with_empty_retired_arrays_migrates() {
        let source = format!(
            "schema_version = 2\nauto_discover = false\nendpoints = []\ncomputers = []\nagents = []\n{}",
            CONTRACT.split("schema_version = 3").nth(1).unwrap()
        );
        let parsed = parse(&source).unwrap();
        assert_eq!(parsed.connections.len(), 2);
    }
    #[test]
    fn schema_v2_with_nonempty_retired_arrays_rejects() {
        for array in ["computers = [{id = \"c\"}]", "agents = [{id = \"a\"}]"] {
            let source = format!(
                "schema_version = 2\nauto_discover = false\nendpoints = []\n{array}\nruntime_agents = []\n"
            );
            // The other required arrays may be absent in these probes —
            // the retired-array rejection is reached first.
            let err = parse(&source).unwrap_err().to_string();
            assert!(
                err.contains("retired") || err.contains("schema_version 2"),
                "{err}"
            );
        }
    }
    #[test]
    fn schema_v2_missing_retired_arrays_rejects() {
        let source = "schema_version = 2\nauto_discover = false\nruntime_agents = []\n";
        assert!(parse(source).is_err());
    }
    #[test]
    fn schema_v2_auto_discover_rejects() {
        let source = "schema_version = 2\nauto_discover = true\nendpoints = []\ncomputers = []\nagents = []\n";
        assert!(parse(source).is_err());
    }
    #[test]
    fn desktop_endpoint_rejects_cross_origin_reference() {
        let source = format!(
            "{}\n[[runtime_agents]]\nid = \"other\"\nname = \"Other\"\ntool = \"shell\"\norigin = \"https://other.example.invalid\"\ndesktop_endpoint = \"nuc\"\n",
            ENDPOINT_CONTRACT.replace("runtime_agents = []\n", "")
        );
        let err = parse(&source).unwrap_err().to_string();
        assert_eq!(
            err,
            "runtime agent desktop_endpoint belongs to another runtime origin"
        );
        // Canonical-equivalent origins still collide; the new check must
        // not make a second binding to the same origin acceptable.
        let same_origin = source.replace(
            "https://other.example.invalid",
            "https://RUNTIME.NUC.EXAMPLE.INVALID:443/",
        );
        assert!(parse(&same_origin)
            .unwrap_err()
            .to_string()
            .contains("duplicates"));
    }
    #[test]
    fn schema_v3_rejects_retired_fields() {
        for field in ["auto_discover = false", "computers = []", "agents = []"] {
            let source = format!("schema_version = 3\n{field}\nruntime_agents = []\n");
            let err = parse(&source).unwrap_err().to_string();
            assert!(err.contains("schema_version 3"), "{err}");
        }
    }
    #[test]
    fn unsupported_schema_version_rejects() {
        assert!(parse("schema_version = 1\n").is_err());
        assert!(parse("schema_version = 4\nruntime_agents = []\n").is_err());
    }
    #[test]
    fn runtime_agents_required_for_v3() {
        assert!(parse("schema_version = 3\n").is_err());
        assert!(parse("schema_version = 3\nruntime_agents = []\n").is_ok());
    }
    #[test]
    fn duplicate_and_invalid_runtime_agents_reject() {
        let base = |body: &str| format!("schema_version = 3\n{body}");
        // duplicate id
        let dup = base(
            "[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid\"\n[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://b.example.invalid\"\n",
        );
        assert!(parse(&dup).is_err());
        // duplicate origin
        let dup = base(
            "[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid\"\n[[runtime_agents]]\nid = \"b\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid/\"\n",
        );
        assert!(parse(&dup).is_err());
        // invalid id / origin / transport / coding_cli
        for body in [
            "[[runtime_agents]]\nid = \"A\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid\"\n",
            "[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"not-a-url\"\n",
            "[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid\"\ndesktop_transport = \"ssh\"\n",
            "[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid\"\ncoding_cli = \"other\"\n",
        ] {
            assert!(parse(&base(body)).is_err(), "{body}");
        }
    }
    #[test]
    fn secrets_and_unknown_fields_reject() {
        for source in [
            "schema_version = 3\npassword = \"x\"\nruntime_agents = []\n",
            "schema_version = 3\nprivate_key = \"x\"\nruntime_agents = []\n",
            "schema_version = 3\nruntime_agents = []\ntoken = \"x\"\n",
            "schema_version = 3\n[[runtime_agents]]\nid = \"a\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://a.example.invalid\"\npassword = \"x\"\n",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
    #[test]
    fn malformed_toml_errors_never_echo_source_text() {
        let secret_line = "token = \"s3cr3t-value-that-must-not-leak\"";
        let source = format!("schema_version = 3\n{secret_line}\nruntime_agents = []\n");
        let err = parse(&source).unwrap_err().to_string();
        assert!(!err.contains("s3cr3t"), "{err}");
        assert!(!err.contains(secret_line), "{err}");
        let malformed = "schema_version = 3\n[[runtime_agents\nid = \"broken\"\n";
        let err = parse(malformed).unwrap_err().to_string();
        assert!(!err.contains("broken"), "{err}");
    }
    #[test]
    fn bounds_enforced() {
        let mut ids = String::from("schema_version = 3\n");
        for i in 0..=RUNTIME_AGENTS_MAX {
            ids.push_str(&format!(
                "[[runtime_agents]]\nid = \"a{i}\"\nname = \"n\"\ntool = \"t\"\norigin = \"https://{i}.example.invalid\"\n"
            ));
        }
        let err = parse(&ids).unwrap_err().to_string();
        assert!(err.contains("runtime_agents"), "{err}");
        let mut eps = String::from("schema_version = 3\nruntime_agents = []\n");
        for i in 0..=ENDPOINTS_MAX {
            eps.push_str(&endpoint_table(&format!("e{i}"), ""));
        }
        let err = parse(&eps).unwrap_err().to_string();
        assert!(err.contains("endpoints"), "{err}");
    }
}
