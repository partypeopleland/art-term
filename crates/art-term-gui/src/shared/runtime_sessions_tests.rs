use super::*;

#[test]
fn gateway_failure_serializes_a_fixed_local_message() {
    let value =
        serde_json::to_value(SessionFailure::from(SessionError::GatewayUnavailable)).unwrap();
    assert_eq!(value["kind"], "gatewayUnavailable");
    assert_eq!(
        value["message"],
        "主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試。"
    );
}
use art_term_core::openab::profiles::HostProfile;
fn binding() -> SessionBinding {
    SessionBinding {
        profile: HostProfile {
            id: "fixture-host".into(),
            revision: 1,
            name: "Fixture".into(),
            base_url: "https://example.invalid".into(),
            credential_present: true,
            metadata: None,
        },
        name: "fixture-shell".into(),
        generation: 1,
    }
}
#[test]
fn preview_ownership_is_revision_scoped_and_capacity_refuses_untracked_create() {
    let shells = Arc::new(Mutex::new(HashMap::new()));
    let profile = binding().profile;
    let registry = OwnedShells {
        shells: shells.clone(),
        profile_id: ownership_key(&profile),
    };
    registry.mark("mcshot-owned").unwrap();
    let mut changed = profile.clone();
    changed.revision += 1;
    assert_ne!(ownership_key(&profile), ownership_key(&changed));
    assert!(!shells
        .lock()
        .unwrap()
        .contains_key(&ownership_key(&changed)));
    assert!(!shells
        .lock()
        .unwrap()
        .get(&ownership_key(&profile))
        .unwrap()
        .contains("mcshot-foreign"));
    for n in 1..64 {
        registry.mark(&format!("mcshot-{n}")).unwrap();
    }
    assert_eq!(
        registry.mark("mcshot-over-capacity"),
        Err(SessionError::Capacity)
    );
    assert!(!shells
        .lock()
        .unwrap()
        .values()
        .any(|names| names.contains("mcshot-over-capacity")));
    registry.unmark("mcshot-owned");
    registry.mark("mcshot-over-capacity").unwrap();
}
struct GuardedPreview;
impl RuntimePreview for GuardedPreview {
    fn mapped<'a>(
        &'a self,
        origin: &'a str,
    ) -> Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>> + Send + 'a>,
    > {
        FakePreview.mapped(origin)
    }
    fn enable<'a>(
        &'a self,
        _: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        panic!("provider must not run without live binding")
    }
    fn screenshot<'a>(
        &'a self,
        _: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        panic!("provider must not run without live binding")
    }
}
struct FakePreview;
impl RuntimePreview for FakePreview {
    fn native_status<'a>(
        &'a self,
        _hosts: &'a RuntimeHosts,
        _conn: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        self.enable("native")
    }
    fn native_screenshot<'a>(
        &'a self,
        _hosts: &'a RuntimeHosts,
        _conn: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        self.screenshot("native")
    }

    fn mapped<'a>(
        &'a self,
        _origin: &'a str,
    ) -> Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>> + Send + 'a>,
    > {
        Box::pin(async { Ok(None) })
    }
    fn enable<'a>(
        &'a self,
        endpoint: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        let endpoint = endpoint.to_owned();
        Box::pin(async move {
            Ok(DesktopStateDto {
                enabled: true,
                message: endpoint,
            })
        })
    }
    fn screenshot<'a>(
        &'a self,
        endpoint: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        let endpoint = endpoint.to_owned();
        Box::pin(async move {
            Ok(ScreenshotDto {
                desktop_id: None,
                desktop_revision: None,
                mime: "image/png".into(),
                base64: String::new(),
                source: endpoint,
                captured_at: 0,
            })
        })
    }
}
/// Lifecycle route-seam fixture. Enrollment tests must install a real live handle.
fn fake_connection(sessions: &RuntimeSessions, route: Option<DesktopRoute>) -> String {
    let id = sessions.id("session");
    let (cancel, _) = watch::channel(false);
    sessions.sessions.lock().unwrap().insert(
        id.clone(),
        Arc::new(Connection {
            handle: Mutex::new(None),
            recovery: Recovery::default(),
            cancel,
            next_generation: AtomicU64::new(2),
            desktop_route: route,
            desktop_enabled: AtomicBool::new(false),
            selected_desktop: Mutex::new(None),
            preview_cancel: watch::channel(0).0,
            preview_operation: Arc::default(),
            owned_shells: Arc::default(),
            closed: AtomicBool::new(false),
            desktop_operation: tokio::sync::Mutex::new(()),
            history_binding: None,
            history: tokio::sync::Mutex::new(None),
            history_cancel: watch::channel(0).0,
            native_interaction: tokio::sync::Mutex::new(()),
            native_enabled: AtomicBool::new(false),
            native_held: AtomicBool::new(false),
            native_uncertain: AtomicBool::new(false),
        }),
    );
    id
}
#[tokio::test]
async fn native_uncertain_without_helper_blocks_successive_user_input_but_not_protocol_route() {
    let sessions = RuntimeSessions::default();
    let id = fake_connection(&sessions, None);
    let connection = sessions.connection(&id).unwrap();
    connection.native_enabled.store(true, Ordering::SeqCst);
    connection.native_uncertain.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        let error = sessions
            .write(&id, b"echo must-not-send\n", false)
            .await
            .unwrap_err();
        assert_eq!(error.kind, SessionError::InputUncertain);
    }
    // With no handle, reaching the original protocol path yields Busy rather than uncertainty.
    assert_eq!(
        sessions
            .write(&id, b"\x1b[1;1R", true)
            .await
            .unwrap_err()
            .kind,
        SessionError::Busy
    );
}
#[tokio::test]
async fn legacy_non_history_session_does_not_enable_native_input_fence() {
    let sessions = RuntimeSessions::default();
    let id = fake_connection(&sessions, None);
    assert!(matches!(
        sessions
            .history_request(&fake_hosts(), &id, HistoryRequest::Scroll { lines: 0 })
            .await
            .unwrap(),
        HistoryReply::Unsupported
    ));
    let connection = sessions.connection(&id).unwrap();
    assert!(!connection.native_enabled.load(Ordering::SeqCst));
    assert!(!connection.native_uncertain.load(Ordering::SeqCst));
    assert_eq!(
        sessions
            .write(&id, b"echo legacy\n", false)
            .await
            .unwrap_err()
            .kind,
        SessionError::Busy
    );
}
/// Failed-store stub: preview-seam tests never touch it, and a real
/// native route on it fails closed through the store error.
fn fake_hosts() -> RuntimeHosts {
    RuntimeHosts {
        store: Err(ApiError::Storage),
        admin: Err(ApiError::Storage),
        connections: None,
        #[cfg(all(debug_assertions, feature = "openab-fixture"))]
        fixture_ca: Ok(None),
    }
}
#[test]
fn repeated_list_reuses_target_without_cumulative_capacity_failure() {
    let sessions = RuntimeSessions::default();
    let initial = sessions.issue(binding(), None, false, None).unwrap().id;
    for generation in 2..=1024 {
        let mut current = binding();
        current.generation = generation;
        assert_eq!(
            sessions.issue(current, None, true, None).unwrap().id,
            initial
        );
    }
    assert_eq!(sessions.targets.lock().unwrap().len(), 1);
}
#[test]
fn replacement_invalidates_old_target_and_missing_sessions_are_pruned() {
    let sessions = RuntimeSessions::default();
    let old = sessions.issue(binding(), None, false, None).unwrap().id;
    let mut replacement = binding();
    replacement.profile.revision += 1;
    let new = sessions.issue(replacement, None, false, None).unwrap().id;
    assert_ne!(old, new);
    assert!(!sessions.targets.lock().unwrap().contains_key(&old));
    sessions.prune_list(&binding().profile, &[]).unwrap();
    assert!(sessions.targets.lock().unwrap().is_empty());
}
#[test]
fn mapped_issue_freezes_desktop_binding_and_unmapped_denies() {
    let sessions = RuntimeSessions::default();
    let mapping = RuntimeMapping {
        route: Some(DesktopRoute::Native),
    };
    let dto = sessions
        .issue(binding(), None, false, Some(mapping))
        .unwrap();
    assert!(dto.desktop_available);
    // A re-issue with NO mapping keeps the frozen binding.
    let same = sessions.issue(binding(), None, false, None).unwrap();
    assert_eq!(same.id, dto.id);
    assert!(same.desktop_available);
    let unmapped = RuntimeSessions::default();
    let dto = unmapped.issue(binding(), None, false, None).unwrap();
    assert!(!dto.desktop_available);
}
#[tokio::test]
async fn unmapped_session_desktop_and_screenshot_are_denied() {
    let sessions = RuntimeSessions::default();
    let id = fake_connection(&sessions, None);
    let preview = FakePreview;
    let hosts = fake_hosts();
    assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
}
#[tokio::test]
async fn mapped_session_enable_then_close_fences_late_calls() {
    mapped_session_enable_then_close_fences_late_calls_route(DesktopRoute::Native).await;
}
#[tokio::test]
async fn mapped_session_enable_then_close_fences_late_calls_native() {
    mapped_session_enable_then_close_fences_late_calls_route(DesktopRoute::Native).await;
}
async fn mapped_session_enable_then_close_fences_late_calls_route(route: DesktopRoute) {
    let sessions = RuntimeSessions::default();
    let id = fake_connection(&sessions, Some(route.clone()));
    let preview = FakePreview;
    let hosts = fake_hosts();
    let state = sessions.desktop(&hosts, &id, true, &preview).await.unwrap();
    assert!(state.enabled);
    // Closed session: enable/disable/screenshot all fail closed,
    // and a late disable never calls the provider again.
    sessions.close(&id).await.unwrap();
    assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
    assert!(sessions
        .desktop(&hosts, &id, false, &preview)
        .await
        .is_err());
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
}
#[tokio::test]
async fn disable_is_local_only_and_screenshot_requires_enabled() {
    disable_is_local_only_and_screenshot_requires_enabled_route(DesktopRoute::Native).await;
}
#[tokio::test]
async fn disable_is_local_only_and_screenshot_requires_enabled_native() {
    disable_is_local_only_and_screenshot_requires_enabled_route(DesktopRoute::Native).await;
}
async fn disable_is_local_only_and_screenshot_requires_enabled_route(route: DesktopRoute) {
    let sessions = RuntimeSessions::default();
    let id = fake_connection(&sessions, Some(route.clone()));
    let preview = FakePreview;
    let hosts = fake_hosts();
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
    sessions.desktop(&hosts, &id, true, &preview).await.unwrap();
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_ok());
    let state = sessions
        .desktop(&hosts, &id, false, &preview)
        .await
        .unwrap();
    assert!(!state.enabled);
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
}
#[tokio::test]
async fn native_route_fails_closed_without_endpoint_fallback() {
    let sessions = RuntimeSessions::default();
    let id = fake_connection(&sessions, Some(DesktopRoute::Native));
    let preview = GuardedPreview;
    let hosts = fake_hosts();
    // The native route binds to the live handle + HostStore only —
    // a failed binding/store check errors without ever consulting
    // the endpoint provider; no SSH fallback exists.
    assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
}
struct DelayedPreview {
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl RuntimePreview for DelayedPreview {
    fn native_status<'a>(
        &'a self,
        _hosts: &'a RuntimeHosts,
        _conn: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        self.enable("native")
    }
    fn native_screenshot<'a>(
        &'a self,
        _hosts: &'a RuntimeHosts,
        _conn: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        self.screenshot("native")
    }

    fn mapped<'a>(
        &'a self,
        origin: &'a str,
    ) -> Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>> + Send + 'a>,
    > {
        FakePreview.mapped(origin)
    }
    fn enable<'a>(
        &'a self,
        endpoint: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        FakePreview.enable(endpoint)
    }
    fn screenshot<'a>(
        &'a self,
        endpoint: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        Box::pin(async move {
            self.started.notify_one();
            self.resume.notified().await;
            FakePreview.screenshot(endpoint).await
        })
    }
}
#[tokio::test]
async fn in_flight_screenshot_is_discarded_after_disable_or_close() {
    in_flight_screenshot_is_discarded_after_disable_or_close_route(DesktopRoute::Native).await;
}
#[tokio::test]
async fn in_flight_screenshot_is_discarded_after_disable_or_close_native() {
    in_flight_screenshot_is_discarded_after_disable_or_close_route(DesktopRoute::Native).await;
}
async fn in_flight_screenshot_is_discarded_after_disable_or_close_route(route: DesktopRoute) {
    for close in [false, true] {
        let sessions = Arc::new(RuntimeSessions::default());
        let id = fake_connection(&sessions, Some(route.clone()));
        let preview = Arc::new(DelayedPreview {
            started: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        let hosts = Arc::new(fake_hosts());
        sessions
            .desktop(&hosts, &id, true, preview.as_ref())
            .await
            .unwrap();
        let task = tokio::spawn({
            let sessions = sessions.clone();
            let preview = preview.clone();
            let hosts = hosts.clone();
            let id = id.clone();
            async move { sessions.screenshot(&hosts, &id, preview.as_ref()).await }
        });
        preview.started.notified().await;
        if close {
            sessions.close(&id).await.unwrap();
        } else {
            sessions
                .desktop(&hosts, &id, false, preview.as_ref())
                .await
                .unwrap();
        }
        preview.resume.notify_one();
        assert!(task.await.unwrap().is_err());
    }
}
#[tokio::test]
async fn shutdown_releases_all_targets() {
    let sessions = RuntimeSessions::default();
    sessions.issue(binding(), None, false, None).unwrap();
    sessions.shutdown().await;
    assert!(sessions.targets.lock().unwrap().is_empty());
}
#[tokio::test]
async fn both_production_routes_reject_absent_handle_before_io() {
    for route in [DesktopRoute::Native] {
        let sessions = RuntimeSessions::default();
        let id = fake_connection(&sessions, Some(route));
        let hosts = fake_hosts();
        assert!(sessions
            .desktop(&hosts, &id, true, &GuardedPreview)
            .await
            .is_err());
        sessions
            .connection(&id)
            .unwrap()
            .desktop_enabled
            .store(true, Ordering::SeqCst);
        assert!(sessions
            .screenshot(&hosts, &id, &GuardedPreview)
            .await
            .is_err());
    }
}
#[test]
fn production_binding_verifier_rejects_revision_identity_and_generation_drift() {
    let expected = binding();
    assert!(
        RuntimeSessions::verify_preview_binding(&expected, &expected, &expected.profile).is_ok()
    );
    for field in 0..3 {
        let mut changed = expected.clone();
        match field {
            0 => changed.generation += 1,
            1 => changed.profile.revision += 1,
            _ => changed.name = "other".into(),
        }
        assert!(
            RuntimeSessions::verify_preview_binding(&expected, &changed, &expected.profile)
                .is_err()
        );
    }
    let mut profile = expected.profile.clone();
    profile.revision += 1;
    assert!(RuntimeSessions::verify_preview_binding(&expected, &expected, &profile).is_err());
}
struct RouteCounters {
    native: AtomicU64,
    endpoint: AtomicU64,
}
impl RuntimePreview for RouteCounters {
    fn mapped<'a>(
        &'a self,
        origin: &'a str,
    ) -> Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Option<RuntimeMapping>>> + Send + 'a>,
    > {
        FakePreview.mapped(origin)
    }
    fn enable<'a>(
        &'a self,
        _: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        self.endpoint.fetch_add(1, Ordering::SeqCst);
        FakePreview.enable("endpoint")
    }
    fn screenshot<'a>(
        &'a self,
        _: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        self.endpoint.fetch_add(1, Ordering::SeqCst);
        FakePreview.screenshot("endpoint")
    }
    fn native_status<'a>(
        &'a self,
        _: &'a RuntimeHosts,
        _: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<DesktopStateDto>> + Send + 'a>>
    {
        self.native.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { anyhow::bail!("native unavailable") })
    }
    fn native_screenshot<'a>(
        &'a self,
        _: &'a RuntimeHosts,
        _: &'a Arc<Connection>,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<ScreenshotDto>> + Send + 'a>> {
        self.native.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { anyhow::bail!("native unavailable") })
    }
}
#[tokio::test]
async fn native_errors_never_call_endpoint_provider() {
    let sessions = RuntimeSessions::default();
    let hosts = fake_hosts();
    let id = fake_connection(&sessions, Some(DesktopRoute::Native));
    let preview = RouteCounters {
        native: AtomicU64::new(0),
        endpoint: AtomicU64::new(0),
    };
    assert!(sessions.desktop(&hosts, &id, true, &preview).await.is_err());
    sessions
        .connection(&id)
        .unwrap()
        .desktop_enabled
        .store(true, Ordering::SeqCst);
    assert!(sessions.screenshot(&hosts, &id, &preview).await.is_err());
    assert_eq!(preview.native.load(Ordering::SeqCst), 2);
    assert_eq!(preview.endpoint.load(Ordering::SeqCst), 0);
}
#[test]
fn both_route_snapshots_survive_reissue_with_a_different_route() {
    for route in [DesktopRoute::Native] {
        let sessions = RuntimeSessions::default();
        let original = format!("{route:?}");
        let dto = sessions
            .issue(
                binding(),
                None,
                false,
                Some(RuntimeMapping { route: Some(route) }),
            )
            .unwrap();
        sessions
            .issue(binding(), None, false, Some(RuntimeMapping { route: None }))
            .unwrap();
        let targets = sessions.targets.lock().unwrap();
        assert_eq!(
            format!(
                "{:?}",
                targets[&dto.id]
                    .desktop
                    .as_ref()
                    .unwrap()
                    .route
                    .as_ref()
                    .unwrap()
            ),
            original
        );
    }
}
