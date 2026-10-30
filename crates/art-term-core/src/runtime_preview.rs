//! Exact origin native Runtime desktop mapping; no fallback transport.
use crate::connections::Connections;

/// Frozen preview route for one runtime origin — resolved once at
/// target/session issuance. A later catalog or profile change can
/// never rebind a live session's preview to a different route.
#[derive(Clone, Debug)]
pub enum DesktopRoute {
    /// Native `desktop_transport = "runtime-mcp"`: the runtime Host's
    /// own authenticated admin API serves status and screenshots for
    /// the live session binding. No SSH endpoint is ever consulted on
    /// this route — there is deliberately no SSH fallback.
    Native,
}

/// Resolved runtime → desktop binding snapshot. Frozen at issue/open
/// time so a later catalog or profile change can never rebind a live
/// session's preview.
#[derive(Clone, Debug)]
pub struct RuntimeMapping {
    /// Frozen capability, independent of Host/Agent classification.
    pub route: Option<DesktopRoute>,
}

/// One effective metadata policy for classification and frozen capabilities.
/// Persisted metadata wins; catalog fallback requires the exact canonical origin.
pub fn resolve_metadata<'a>(
    local: Option<&'a crate::connections::ConnectionMetadata>,
    connections: Option<&'a Connections>,
    origin: &str,
) -> crate::connections::ConnectionMetadata {
    local
        .cloned()
        .or_else(|| {
            connections
                .and_then(|c| c.connection(origin))
                .map(|c| c.metadata.clone())
        })
        .unwrap_or_default()
}
pub fn mapping_from_metadata(metadata: &crate::connections::ConnectionMetadata) -> RuntimeMapping {
    RuntimeMapping {
        route: metadata
            .desktop_transport
            .as_deref()
            .filter(|v| *v == "runtime-mcp")
            .map(|_| DesktopRoute::Native),
    }
}
pub fn mapping_for(connections: &Connections, canonical_origin: &str) -> Option<RuntimeMapping> {
    connections.connection(canonical_origin).map(|_| {
        mapping_from_metadata(&resolve_metadata(None, Some(connections), canonical_origin))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::tests::{CONTRACT, ENDPOINT_CONTRACT};

    fn contract() -> Connections {
        crate::connections::parse(CONTRACT).unwrap()
    }

    /// Endpoint `runtime_origin` migrates to the Native route; the
    /// declared native agent maps Native; everything else is denied —
    /// capability never depends on the Computer/Agent classification.
    #[test]
    fn exact_origin_maps_and_everything_else_is_denied() {
        let endpoint = crate::connections::parse(ENDPOINT_CONTRACT).unwrap();
        let mapped = mapping_for(&endpoint, "https://runtime.nuc.example.invalid/").unwrap();
        assert!(matches!(mapped.route, Some(DesktopRoute::Native)));

        let connections = contract();
        let hana = mapping_for(&connections, "https://runtime.hana.example.invalid/").unwrap();
        assert!(hana.route.is_none());
        let nuc = mapping_for(&connections, "https://runtime.nuc.example.invalid/").unwrap();
        assert!(matches!(nuc.route, Some(DesktopRoute::Native)));

        for denied in [
            "",
            "https://other.example.invalid", // undeclared host
            "https://runtime.nuc.example.invalid/extra", // added path
            "HTTPS://RUNTIME.NUC.EXAMPLE.INVALID/", // not canonicalized
            "nuc",                           // label, not an origin
        ] {
            assert!(
                mapping_for(&connections, denied).is_none()
                    && mapping_for(&endpoint, denied).is_none(),
                "{denied:?} must never map"
            );
        }
    }

    /// An agent `desktop_endpoint` migrates to the Native route without the
    /// origin being an endpoint `runtime_origin` — NUC-style computers
    /// need no Agent label.
    #[test]
    fn agent_desktop_endpoint_migrates_to_native() {
        let source = format!(
            "{}\n\n[[runtime_agents]]\nid = \"desk-agent\"\nname = \"Desk\"\ntool = \"shell\"\norigin = \"https://other.example.invalid\"\ndesktop_endpoint = \"nuc\"\n",
            ENDPOINT_CONTRACT
                .replace("runtime_agents = []\n", "")
                .replace("runtime_origin = \"https://runtime.nuc.example.invalid/\"\n", "")
        );
        let parsed = crate::connections::parse(&source).unwrap();
        let mapped = mapping_for(&parsed, "https://other.example.invalid/").unwrap();
        assert!(matches!(mapped.route, Some(DesktopRoute::Native)));
        assert!(mapping_for(&parsed, "https://runtime.nuc.example.invalid/").is_none());
    }

    #[test]
    fn unmapped_legacy_endpoint_has_no_native_route() {
        // A legacy endpoint without an origin never maps by hostname.
        let connections = contract();
        assert!(mapping_for(&connections, "https://unrelated.example.invalid/").is_none());
        let provider_only = format!(
            "schema_version = 3\nruntime_agents = []\n[[endpoints]]\nid = \"p\"\nhost = \"p.example.invalid\"\nport = 22\nusername = \"alice\"\nnetwork = \"tailnet\"\nexpected_host_key_algorithm = \"ssh-ed25519\"\nexpected_host_key = \"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKurq6urq6urq6urq6urq6urq6urq6urq6urq6urq6ur\"\nexpected_host_key_sha256 = \"SHA256:mEwqXdTPoHnFTvEV3e8U+5RuFC2tSO9zs8CTJPIXq/w\"\ndesktop_provider = \"art-term-hands\"\n"
        );
        let parsed = crate::connections::parse(&provider_only).unwrap();
        assert!(mapping_for(&parsed, "https://any.example.invalid/").is_none());
    }
}
