//! Dedicated temporary-Shell preview transport over the official PTY API.
//!
//! The unmodified runtime only moves Shell input/output, so a screenshot is
//! a short-lived App-owned Shell: a random reserved name is created, the
//! fixed launcher command is sent once, the helper JSON is framed by unique
//! nonce markers, and the Shell is deleted on success, error, or cancel.
//! The work Shell is never touched and no MCP credential crosses the PTY —
//! the launcher reads the host-side protected settings itself.

use super::{
    api::{AdminClient, SessionGrant},
    profiles::HostSnapshot,
    secret::Secret,
    session::{OutputEvent, SessionBinding, SessionError, SessionHandle},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

/// Reserved App-owned Shell name prefix (`mcshot-` + 24 lowercase hex ≤ 32).
pub const SHELL_PREFIX: &str = "mcshot-";
/// Fixed launcher contract shared with the host installer. The nonce is a
/// framing tag, never a credential; the command line itself contains no
/// marker text, so a PTY echo can never forge one.
const LAUNCHER: &str = "python3 \"$HOME/.local/share/art-term/openab-pty/preview.py\"";
/// Total PTY output bound for one operation (contract: ≤ 16 MiB).
const STREAM_LIMIT: usize = 16 * 1024 * 1024;
/// One bounded deadline for the whole create→attach→collect operation.
const OPERATION_DEADLINE: Duration = Duration::from_secs(60);
/// One bounded DELETE attempt; cleanup failure keeps the name owned.
const CLEANUP_DEADLINE: Duration = Duration::from_secs(10);

/// `mcshot-` + 24 random hex — a valid `[a-z0-9-]{1,32}` name.
pub fn new_shell_name() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("{SHELL_PREFIX}{}", &raw[..24])
}
fn new_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
/// Name format only. Ownership and visibility require an explicit registry entry.
pub fn is_preview_name(name: &str) -> bool {
    name.starts_with(SHELL_PREFIX)
}
fn launcher_command(action: &str, nonce: &str) -> String {
    format!("{LAUNCHER} {action} {nonce}\n")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Screenshot,
    Status,
}
impl Mode {
    fn action(self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::Status => "status",
        }
    }
}

/// App-side ownership registry. A marked name is hidden from the
/// selectable session list; `unmark` runs only after a confirmed DELETE.
/// A failed cleanup leaves the mark so the name stays owned and retryable
/// instead of being silently forgotten.
pub trait ShellOwnership: Send + Sync {
    fn mark(&self, name: &str) -> Result<(), SessionError>;
    fn unmark(&self, name: &str);
}

/// A validated helper screenshot: same contract as the retired admin path —
/// `image_base64` is the exact wire payload proven to decode to `bytes`.
#[derive(Debug)]
pub struct ShotImage {
    pub mime: String,
    pub image_base64: String,
    pub width: u32,
    pub height: u32,
}

/// Bounded cleanup of exactly the Shell this module created — never any
/// other session. Armed until a DELETE succeeds; on in-scope exits it runs
/// inline, and if the `run` future itself is dropped the armed guard still
/// spawns one bounded attempt on the current runtime.
struct Cleanup {
    api: AdminClient,
    host: Option<HostSnapshot>,
    name: String,
    ownership: Arc<dyn ShellOwnership>,
    done: bool,
}
impl Cleanup {
    async fn delete_once(&mut self) {
        let Some(host) = self.host.as_ref() else {
            return;
        };
        if matches!(
            tokio::time::timeout(CLEANUP_DEADLINE, self.api.delete(&host, &self.name)).await,
            Ok(Ok(())) | Ok(Err(super::api::ApiError::SessionMissing))
        ) {
            self.ownership.unmark(&self.name);
            self.done = true;
            self.host = None;
        }
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let Some(host) = self.host.take() else {
            return;
        };
        let api = self.api.clone();
        let name = self.name.clone();
        let ownership = self.ownership.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if matches!(
                    tokio::time::timeout(CLEANUP_DEADLINE, api.delete(&host, &name)).await,
                    Ok(Ok(())) | Ok(Err(super::api::ApiError::SessionMissing))
                ) {
                    ownership.unmark(&name);
                }
            });
        }
    }
}

/// One preview operation: create the owned Shell, attach with its minted
/// token, send the fixed launcher, collect the framed JSON reply, then
/// delete the Shell. `attach` supplies the transport (fixture-aware in the
/// GUI); the token is used only on this channel and dies with it.
/// `attach` is called as `attach(binding, token, since, local_generation)`
/// with `since = 0` and the fixed local generation `1` that `collect`
/// acknowledges with — fixture-aware transports are supplied by the GUI.
pub async fn run<F, Fut>(
    api: &AdminClient,
    host: HostSnapshot,
    mode: Mode,
    ownership: Arc<dyn ShellOwnership>,
    mut cancel: watch::Receiver<u64>,
    attach: F,
) -> Result<serde_json::Value, SessionError>
where
    F: FnOnce(SessionBinding, Secret, u64, u64) -> Fut,
    Fut: std::future::Future<Output = Result<SessionHandle, SessionError>>,
{
    let name = new_shell_name();
    ownership.mark(&name)?;
    let mut cleanup = Cleanup {
        api: api.clone(),
        host: Some(host),
        name,
        ownership,
        done: false,
    };
    let operation = tokio::time::timeout(OPERATION_DEADLINE, async {
        let host = cleanup.host.as_ref().ok_or(SessionError::Internal)?;
        let grant: SessionGrant = match cleanup.api.create(host, &cleanup.name).await {
            Ok(grant) => grant,
            Err(super::api::ApiError::StateChanged) => {
                // Explicit conflict proves this call did not create the existing Shell.
                cleanup.ownership.unmark(&cleanup.name);
                cleanup.done = true;
                return Err(SessionError::StateChanged);
            }
            Err(error) => return Err(SessionError::from(error)),
        };
        let binding = SessionBinding::from_grant(host.profile.clone(), &grant)?;
        let handle = attach(binding, grant.token, 0, 1).await?;
        handle.preview_flow();
        let collected = collect(&handle, mode).await;
        handle.shutdown().await;
        collected
    });
    let result = tokio::select! { biased;
        _ = cancel.changed() => Err(SessionError::Cancelled),
        result = operation => result.unwrap_or(Err(SessionError::Timeout)),
    };
    let _ = cleanup.delete_once().await;
    result
}

/// Sensitive configure exchange on an exec-only, App-owned temporary Shell.
/// No secret is sent before the receiver has disabled PTY echo/canonical mode.
pub async fn configure<F, Fut>(
    api: &AdminClient,
    host: HostSnapshot,
    binding: &str,
    payload: zeroize::Zeroizing<String>,
    ownership: Arc<dyn ShellOwnership>,
    mut cancel: watch::Receiver<u64>,
    attach: F,
) -> Result<serde_json::Value, SessionError>
where
    F: FnOnce(SessionBinding, Secret, u64, u64) -> Fut,
    Fut: std::future::Future<Output = Result<SessionHandle, SessionError>>,
{
    use base64::Engine;
    validate_binding(binding)?;
    let name = new_shell_name();
    ownership.mark(&name)?;
    let mut cleanup = Cleanup {
        api: api.clone(),
        host: Some(host),
        name,
        ownership,
        done: false,
    };
    let operation = async {
        let host = cleanup.host.as_ref().ok_or(SessionError::Internal)?;
        let grant = match api.create(host, &cleanup.name).await {
            Ok(g) => g,
            Err(super::api::ApiError::StateChanged) => {
                cleanup.ownership.unmark(&cleanup.name);
                cleanup.done = true;
                return Err(SessionError::StateChanged);
            }
            Err(e) => return Err(e.into()),
        };
        let handle = attach(
            SessionBinding::from_grant(host.profile.clone(), &grant)?,
            grant.token,
            0,
            1,
        )
        .await?;
        handle.preview_flow();
        let exchange = async {
            let nonce = new_nonce();
            let command = format!("exec python3 \"$HOME/.local/share/art-term/tools/instance_bridge.py\" control --binding {binding} --nonce {nonce}\n");
            handle.write(command.as_bytes())?;
            let ready = format!("ARTTERM_TOOLS_READY:{nonce}");
            let result = format!("ARTTERM_TOOLS_RESULT:{nonce}:");
            let encoded = zeroize::Zeroizing::new(
                base64::engine::general_purpose::STANDARD.encode(payload.as_bytes()),
            );
            if encoded.len() > 16384 {
                return Err(SessionError::Capacity);
            }
            let mut buffer = Vec::new();
            let mut offset = 0;
            let mut sent = false;
            loop {
                let poll = handle.poll()?;
                let mut answer = None;
                for event in poll.events {
                    match event {
                        OutputEvent::Bytes {
                            bytes, start, end, ..
                        } => {
                            if start != offset || end != start + bytes.len() as u64 {
                                return Err(SessionError::Protocol);
                            }
                            offset = end;
                            buffer.extend_from_slice(&bytes);
                            if buffer.len() > 65536 {
                                return Err(SessionError::Capacity);
                            }
                            while let Some(n) = buffer.iter().position(|b| *b == b'\n') {
                                let line: Vec<_> = buffer.drain(..=n).collect();
                                let line = std::str::from_utf8(&line)
                                    .map_err(|_| SessionError::Protocol)?
                                    .trim_end_matches(['\r', '\n']);
                                if line == ready && !sent {
                                    let mut frame = zeroize::Zeroizing::new(Vec::with_capacity(
                                        encoded.len() + 4,
                                    ));
                                    frame.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
                                    frame.extend_from_slice(encoded.as_bytes());
                                    handle.write(&frame)?;
                                    sent = true;
                                } else if let Some(json) = line.strip_prefix(&result) {
                                    if !sent {
                                        return Err(SessionError::Protocol);
                                    }
                                    answer = Some(
                                        serde_json::from_str(json)
                                            .map_err(|_| SessionError::Protocol)?,
                                    );
                                }
                            }
                        }
                        OutputEvent::Gap { .. } => return Err(SessionError::Protocol),
                        _ => {}
                    }
                }
                if poll.batch_id != 0 {
                    handle.ack(poll.batch_id, 1)?;
                }
                if let Some(answer) = answer {
                    return Ok(answer);
                }
                if let Some(ended) = poll.ended {
                    return Err(ended);
                }
                handle.wait_output().await;
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(10), exchange)
            .await
            .map_err(|_| SessionError::Timeout)?;
        handle.shutdown().await;
        result
    };
    let result = tokio::select! {
        _ = cancel.changed() => Err(SessionError::Cancelled),
        r = tokio::time::timeout(OPERATION_DEADLINE, operation) => r.map_err(|_| SessionError::Timeout).and_then(|r| r),
    };
    let _ = cleanup.delete_once().await;
    result
}
fn validate_binding(binding: &str) -> Result<(), SessionError> {
    super::api::validate_session_name(binding).map_err(Into::into)
}

/// Read bounded framed output until the END marker; any gap, offset jump,
/// stream end, or cancellation before END discards the whole result.
async fn collect(handle: &SessionHandle, mode: Mode) -> Result<serde_json::Value, SessionError> {
    let nonce = new_nonce();
    let mut framing = Framing::new(&nonce);
    let mut offset = 0u64;
    handle.write(launcher_command(mode.action(), &nonce).as_bytes())?;
    loop {
        let poll = handle.poll()?;
        for event in poll.events {
            match event {
                OutputEvent::Bytes {
                    bytes, start, end, ..
                } => {
                    if start != offset
                        || end
                            != start
                                .checked_add(bytes.len() as u64)
                                .ok_or(SessionError::Protocol)?
                    {
                        return Err(SessionError::Protocol);
                    }
                    offset = end;
                    framing.feed(&bytes)?;
                }
                OutputEvent::Gap { .. } => return Err(SessionError::Protocol),
                OutputEvent::Notice { .. } | OutputEvent::TtlWarning { .. } => {}
            }
        }
        if poll.batch_id != 0 {
            handle.ack(poll.batch_id, 1)?;
        }
        if framing.complete() {
            return framing.finish();
        }
        if let Some(ended) = poll.ended {
            return Err(ended);
        }
        handle.wait_output().await;
    }
}

/// Strict line framing: a BEGIN line is followed by exactly one JSON line
/// and one END line carrying the same nonce and exit code. Marker text
/// must occupy a whole line (a `\r` from PTY translation is stripped), so
/// the echoed command — which never contains `ARTTERM_PREVIEW_` — cannot
/// satisfy it. Anything else inside the framed region fails the capture.
struct Framing {
    nonce: String,
    buffer: Vec<u8>,
    total: usize,
    state: FramingState,
    json: Option<serde_json::Value>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FramingState {
    Seeking,
    Json,
    End,
    Done,
}
impl Framing {
    fn new(nonce: &str) -> Self {
        Self {
            nonce: nonce.to_string(),
            buffer: Vec::new(),
            total: 0,
            state: FramingState::Seeking,
            json: None,
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<(), SessionError> {
        self.total = self
            .total
            .checked_add(bytes.len())
            .ok_or(SessionError::Capacity)?;
        if self.total > STREAM_LIMIT {
            return Err(SessionError::Capacity);
        }
        self.buffer.extend_from_slice(bytes);
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&line)?;
            if self.state == FramingState::Done {
                self.buffer.clear();
                break;
            }
        }
        if self.buffer.len() > STREAM_LIMIT {
            return Err(SessionError::Capacity);
        }
        Ok(())
    }
    fn line(&mut self, line: &[u8]) -> Result<(), SessionError> {
        match self.state {
            FramingState::Seeking => {
                if line == format!("ARTTERM_PREVIEW_BEGIN:{}", self.nonce).as_bytes() {
                    self.state = FramingState::Json;
                }
            }
            FramingState::Json => {
                let value: serde_json::Value =
                    serde_json::from_slice(line).map_err(|_| SessionError::Protocol)?;
                if !value.is_object() {
                    return Err(SessionError::Protocol);
                }
                self.json = Some(value);
                self.state = FramingState::End;
            }
            FramingState::End => {
                let text = std::str::from_utf8(line).map_err(|_| SessionError::Protocol)?;
                let code = text
                    .strip_prefix(&format!("ARTTERM_PREVIEW_END:{}:", self.nonce))
                    .ok_or(SessionError::Protocol)?;
                if code != "0" {
                    return Err(SessionError::Protocol);
                }
                self.state = FramingState::Done;
            }
            FramingState::Done => {}
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.state == FramingState::Done
    }
    fn finish(self) -> Result<serde_json::Value, SessionError> {
        if !self.complete() {
            return Err(SessionError::Protocol);
        }
        self.json.ok_or(SessionError::Protocol)
    }
}

/// Decode + fully validate the helper screenshot JSON: PNG/JPEG MIME,
/// base64/dimension/allocation caps via `crate::image`, and `bytes` must
/// equal the decoded size. Runs the decode off the async worker.
pub async fn decode_screenshot(json: serde_json::Value) -> Result<ShotImage, SessionError> {
    let mime = json
        .get("mime")
        .and_then(|v| v.as_str())
        .filter(|m| matches!(*m, "image/png" | "image/jpeg"))
        .ok_or(SessionError::Protocol)?
        .to_string();
    let encoded = json
        .get("image_base64")
        .and_then(|v| v.as_str())
        .ok_or(SessionError::Protocol)?
        .to_string();
    let reported = json
        .get("bytes")
        .and_then(|v| v.as_u64())
        .ok_or(SessionError::Protocol)?;
    let wire = encoded.clone();
    let image = tokio::task::spawn_blocking(move || crate::image::validate_base64(&mime, &encoded))
        .await
        .map_err(|_| SessionError::Protocol)?
        .map_err(|_| SessionError::Protocol)?;
    if image.bytes.len() as u64 != reported {
        return Err(SessionError::Protocol);
    }
    Ok(ShotImage {
        mime: image.mime,
        image_base64: wire,
        width: image.width,
        height: image.height,
    })
}

/// Helper status line: `enabled` must be an explicit boolean.
pub fn decode_status(json: &serde_json::Value) -> Result<bool, SessionError> {
    json.get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or(SessionError::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_names_and_launcher_match_the_fixed_contract() {
        for _ in 0..64 {
            let name = new_shell_name();
            assert_eq!(name.len(), SHELL_PREFIX.len() + 24);
            assert!(is_preview_name(&name));
            assert!(super::super::api::validate_session_name(&name).is_ok());
        }
        assert!(!is_preview_name("work-shell"));
        let nonce = new_nonce();
        assert_eq!(nonce.len(), 32);
        assert!(nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        let command = launcher_command(Mode::Screenshot.action(), &nonce);
        assert_eq!(
            command,
            format!(
                "python3 \"$HOME/.local/share/art-term/openab-pty/preview.py\" screenshot {nonce}\n"
            )
        );
        assert!(!command.contains("ARTTERM_PREVIEW"));
    }
    #[test]
    fn framing_accepts_segmented_crlf_output_and_rejects_echo() {
        let nonce = "a".repeat(32);
        let mut framing = Framing::new(&nonce);
        let output = format!(
            "python3 \"$HOME/.local/share/art-term/openab-pty/preview.py\" screenshot {nonce}\r\n$ \r\nARTTERM_PREVIEW_BEGIN:{nonce}\r\n{{\"enabled\":true}}\r\nARTTERM_PREVIEW_END:{nonce}:0\r\n$ "
        );
        let mut cut = output.clone().into_bytes();
        while !framing.complete() && !cut.is_empty() {
            let n = 1.min(cut.len());
            let head: Vec<u8> = cut.drain(..n).collect();
            framing.feed(&head).unwrap();
        }
        assert_eq!(framing.finish().unwrap()["enabled"], true);
    }
    #[test]
    fn framing_rejects_foreign_nonce_bad_exit_extra_lines_and_incomplete() {
        let nonce = "b".repeat(32);
        for output in [
            format!("ARTTERM_PREVIEW_BEGIN:{}\r\n{{\"enabled\":true}}\r\nARTTERM_PREVIEW_END:{}:0\r\n", "c".repeat(32), nonce),
            format!("ARTTERM_PREVIEW_BEGIN:{nonce}\r\n{{\"enabled\":true}}\r\nARTTERM_PREVIEW_END:{nonce}:1\r\n"),
            format!("ARTTERM_PREVIEW_BEGIN:{nonce}\r\n{{\"enabled\":true}}\r\nextra\r\nARTTERM_PREVIEW_END:{nonce}:0\r\n"),
            format!("ARTTERM_PREVIEW_BEGIN:{nonce}\r\nnot-json\r\nARTTERM_PREVIEW_END:{nonce}:0\r\n"),
            format!("ARTTERM_PREVIEW_BEGIN:{nonce}\r\n[1,2]\r\nARTTERM_PREVIEW_END:{nonce}:0\r\n"),
        ] {
            let mut framing = Framing::new(&nonce);
            assert!(
                framing.feed(output.as_bytes()).is_err()
                    || !framing.complete(),
                "accepted {output:?}"
            );
        }
        let mut framing = Framing::new(&nonce);
        framing
            .feed(format!("ARTTERM_PREVIEW_BEGIN:{nonce}\r\n{{\"enabled\":true}}\r\n").as_bytes())
            .unwrap();
        assert!(!framing.complete());
        assert!(framing.finish().is_err());
        // A lone echoed marker substring inside another line never counts.
        let mut framing = Framing::new(&nonce);
        framing
            .feed(format!("echo ARTTERM_PREVIEW_BEGIN:{nonce} rest\r\n").as_bytes())
            .unwrap();
        assert!(!framing.complete());
    }

    fn tiny_png() -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        ::image::RgbImage::new(2, 2)
            .write_to(&mut bytes, ::image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[tokio::test]
    async fn decode_screenshot_validates_envelope_and_preserves_wire_bytes() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let png = tiny_png();
        let encoded = STANDARD.encode(&png);
        let ok = serde_json::json!({
            "mime": "image/png",
            "image_base64": encoded,
            "bytes": png.len(),
        });
        let shot = decode_screenshot(ok).await.unwrap();
        assert_eq!(shot.mime, "image/png");
        assert_eq!(shot.image_base64, encoded);
        assert_eq!((shot.width, shot.height), (2, 2));
        // The reported byte count must equal the decoded size exactly.
        for reported in [png.len() as u64 - 1, png.len() as u64 + 1] {
            let bad = serde_json::json!({
                "mime": "image/png",
                "image_base64": encoded,
                "bytes": reported,
            });
            assert!(
                decode_screenshot(bad).await.is_err(),
                "accepted bytes={reported}"
            );
        }
        // Wrong MIME, invalid base64, and mistyped/missing fields fail.
        for bad in [
            serde_json::json!({"mime": "image/gif", "image_base64": encoded, "bytes": png.len()}),
            serde_json::json!({"mime": "image/png", "image_base64": "!!!", "bytes": 3}),
            serde_json::json!({"mime": "image/png", "image_base64": encoded, "bytes": "oops"}),
            serde_json::json!({"image_base64": encoded, "bytes": png.len()}),
            serde_json::json!({"mime": "image/png", "bytes": png.len()}),
            serde_json::json!({"mime": "image/png", "image_base64": encoded}),
        ] {
            assert!(
                decode_screenshot(bad.clone()).await.is_err(),
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn decode_status_accepts_only_an_explicit_boolean() {
        assert!(decode_status(&serde_json::json!({"enabled": true})).unwrap());
        assert!(!decode_status(&serde_json::json!({"enabled": false})).unwrap());
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"enabled": "true"}),
            serde_json::json!({"enabled": 1}),
            serde_json::json!({"enabled": null}),
        ] {
            assert!(decode_status(&bad).is_err(), "accepted {bad}");
        }
    }
}
