//! Native-owned, bounded Host history helper channel. Work PTY is never touched.
use super::{
    api::AdminClient,
    profiles::HostSnapshot,
    screenshot::ShellOwnership,
    secret::Secret,
    session::{SessionBinding, SessionError, SessionHandle},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum HistoryRequest {
    Viewport,
    Older {
        instance: String,
        epoch: u64,
        before: usize,
        limit: usize,
    },
    Ack {
        instance: String,
        epoch: u64,
    },
    Keep {
        instance: String,
        epoch: u64,
    },
    Close,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryLine {
    pub id: usize,
    pub text: String,
    pub styles: Vec<[i64; 5]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HistoryReply {
    Page {
        instance: String,
        epoch: u64,
        columns: usize,
        rows: usize,
        row_count: usize,
        start: usize,
        end: usize,
        has_older: bool,
        records: Vec<HistoryLine>,
    },
    Ack {
        op: String,
        instance: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        epoch: Option<u64>,
    },
    Unsupported,
    Error {
        error: String,
    },
}
pub fn key_from_name(name: &str) -> Option<String> {
    let k = name.strip_prefix("th2-").or_else(|| name.strip_prefix("th-"))?;
    (k.len() == 24
        && k.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then(|| k.to_string())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryVersion {
    Legacy,
    Paged,
}
impl HistoryVersion {
    pub fn from_name(name: &str) -> Option<Self> {
        key_from_name(name)?;
        Some(if name.starts_with("th2-") { Self::Paged } else { Self::Legacy })
    }
    pub fn wrapper_script(self) -> &'static str {
        match self {
            Self::Legacy => "tmux-shell.py",
            Self::Paged => "tmux-v2/tmux-shell.py",
        }
    }
    /// Only fresh work Shells use the persistent tools overlay.
    pub fn bootstrap_command(self, key: &str) -> Result<String, SessionError> {
        if !hex(key, 24) {
            return Err(SessionError::Protocol);
        }
        let fallback = format!("exec python3 \"$HOME/.local/share/art-term/openab-pty/{}\" {key}", self.wrapper_script());
        Ok(if self == Self::Paged {
            format!("if [ -f \"$HOME/.local/share/art-term/tools/tmux-shell.py\" ]; then exec python3 \"$HOME/.local/share/art-term/tools/tmux-shell.py\" {key}; else {fallback}; fi\n")
        } else {
            format!("{fallback}\n")
        })
    }
    fn history_script(self) -> &'static str {
        match self {
            Self::Legacy => "tmux-history.cjs",
            Self::Paged => "tmux-v2/tmux-history.cjs",
        }
    }
}
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);
const CLEANUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
const FRAME_CAP: usize = 256 * 1024;
const SAFE_INT: u64 = 9_007_199_254_740_991;
fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
struct Cleanup {
    api: AdminClient,
    host: Option<HostSnapshot>,
    name: String,
    ownership: Arc<dyn ShellOwnership>,
}
impl Cleanup {
    async fn delete(&mut self) {
        if let Some(host) = self.host.as_ref() {
            if matches!(
                tokio::time::timeout(CLEANUP_DEADLINE, self.api.delete(host, &self.name)).await,
                Ok(Ok(())) | Ok(Err(super::api::ApiError::SessionMissing))
            ) {
                self.ownership.unmark(&self.name);
                self.host = None;
            }
        }
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let Some(host) = self.host.take() else { return };
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
/// Contains the helper token only in its native SessionHandle. Not serializable.
pub struct HistoryChannel {
    handle: Option<SessionHandle>,
    cleanup: Cleanup,
    nonce: String,
    instance: String,
    key: String,
    offset: u64,
    buffer: Vec<u8>,
    pre_ready: usize,
    next_id: u64,
    in_flight: bool,
}
impl HistoryChannel {
    /// Reserved owned Shell name; never a credential or ownership proof.
    pub fn helper_name(&self) -> &str {
        &self.cleanup.name
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub async fn open<F, Fut>(
        api: &AdminClient,
        host: HostSnapshot,
        key: &str,
        version: HistoryVersion,
        ownership: Arc<dyn ShellOwnership>,
        attach: F,
    ) -> Result<Self, SessionError>
    where
        F: FnOnce(SessionBinding, Secret, u64, u64) -> Fut,
        Fut: std::future::Future<Output = Result<SessionHandle, SessionError>>,
    {
        if !hex(key, 24) {
            return Err(SessionError::Protocol);
        }
        let name = super::screenshot::new_shell_name();
        ownership.mark(&name)?;
        let mut channel = Self {
            handle: None,
            cleanup: Cleanup {
                api: api.clone(),
                host: Some(host),
                name,
                ownership,
            },
            nonce: uuid::Uuid::new_v4().simple().to_string(),
            instance: String::new(),
            key: key.into(),
            offset: 0,
            buffer: Vec::new(),
            pre_ready: 0,
            next_id: 1,
            in_flight: false,
        };
        let result = tokio::time::timeout(DEADLINE, async {
            let host = channel
                .cleanup
                .host
                .as_ref()
                .ok_or(SessionError::Internal)?;
            let grant = match api.create(host, &channel.cleanup.name).await {
                Ok(g) => g,
                Err(super::api::ApiError::StateChanged) => {
                    channel.cleanup.ownership.unmark(&channel.cleanup.name);
                    channel.cleanup.host = None;
                    return Err(SessionError::StateChanged);
                }
                Err(e) => return Err(e.into()),
            };
            if grant.session != channel.cleanup.name {
                return Err(SessionError::Protocol);
            }
            let binding = SessionBinding::from_grant(host.profile.clone(), &grant)?;
            channel.handle = Some(attach(binding, grant.token, 0, 1).await?);
            channel
                .handle
                .as_ref()
                .ok_or(SessionError::Internal)?
                .write(
                    format!(
                    "exec \"$HOME/.local/share/art-term/openab-pty/node\" \"$HOME/.local/share/art-term/openab-pty/{}\" {} {}\n",
                    version.history_script(), channel.key, channel.nonce
                )
                    .as_bytes(),
                )?;
            let value = channel.receive(true).await?;
            if value["type"] == "error" {
                if value["requestId"].as_u64() != Some(0)
                    || !value["instance"].as_str().is_some_and(|s| hex(s, 32))
                {
                    return Err(SessionError::Protocol);
                }
                return Err(match value["kind"].as_str() {
                    Some("unavailable") => SessionError::Missing,
                    Some("unsupported") => SessionError::UpgradeRequired,
                    Some("busy") => SessionError::Busy,
                    Some("limit") => SessionError::Capacity,
                    _ => SessionError::Protocol,
                });
            }
            if value["type"] != "ready"
                || value["requestId"].as_u64() != Some(0)
                || value["key"].as_str() != Some(&channel.key)
            {
                return Err(SessionError::Protocol);
            }
            let instance = value["instance"]
                .as_str()
                .filter(|s| hex(s, 32))
                .ok_or(SessionError::Protocol)?;
            channel.instance = instance.into();
            Ok(())
        })
        .await
        .unwrap_or(Err(SessionError::Timeout));
        match result {
            Ok(()) => Ok(channel),
            Err(e) => {
                channel.close().await;
                Err(e)
            }
        }
    }
    pub async fn request(&mut self, request: HistoryRequest) -> Result<HistoryReply, SessionError> {
        if self.handle.is_none() {
            return Err(SessionError::Cancelled);
        }
        // A dropped request future is never replayed; only closing this helper is safe.
        if self.in_flight {
            return Err(SessionError::StateChanged);
        }
        let (op, epoch) = match &request {
            HistoryRequest::Viewport => ("viewport", None),
            HistoryRequest::Older {
                instance,
                epoch,
                before,
                limit,
            } => {
                self.check_epoch(instance, *epoch)?;
                if *limit == 0 || *limit > 256 || *before > 6000 {
                    return Err(SessionError::Protocol);
                }
                ("older", Some(*epoch))
            }
            HistoryRequest::Ack { instance, epoch } => {
                self.check_epoch(instance, *epoch)?;
                ("ack", Some(*epoch))
            }
            HistoryRequest::Keep { instance, epoch } => {
                self.check_epoch(instance, *epoch)?;
                ("keep", Some(*epoch))
            }
            HistoryRequest::Close => ("close", None),
        };
        let id = self.next_id;
        if id > SAFE_INT {
            return Err(SessionError::Capacity);
        }
        self.next_id += 1;
        let mut wire = serde_json::to_value(&request).map_err(|_| SessionError::Protocol)?;
        let obj = wire.as_object_mut().ok_or(SessionError::Protocol)?;
        obj.remove("instance");
        obj.insert("v".into(), 1.into());
        obj.insert("nonce".into(), self.nonce.clone().into());
        obj.insert("requestId".into(), id.into());
        let bytes = format!("{wire}\n");
        if bytes.len() > 8192 {
            return Err(SessionError::Capacity);
        }
        self.in_flight = true;
        let result = tokio::time::timeout(DEADLINE, async {
            self.handle
                .as_ref()
                .ok_or(SessionError::Cancelled)?
                .write(bytes.as_bytes())?;
            let reply = self.receive(false).await?;
            self.validate_reply(reply, &request, op, epoch, id)
        })
        .await
        .unwrap_or(Err(SessionError::Timeout));
        self.in_flight = false;
        if result.is_err() || matches!(request, HistoryRequest::Close) {
            self.close().await;
        }
        result
    }
    fn check_epoch(&self, instance: &str, epoch: u64) -> Result<(), SessionError> {
        if instance != self.instance || epoch == 0 || epoch > SAFE_INT {
            Err(SessionError::Protocol)
        } else {
            Ok(())
        }
    }
    pub async fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown().await;
        }
        self.cleanup.delete().await;
    }
    async fn receive(&mut self, ready: bool) -> Result<serde_json::Value, SessionError> {
        loop {
            if let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
                if pos + 1 > if ready { 16384 } else { FRAME_CAP } {
                    return Err(SessionError::Capacity);
                }
                let mut line: Vec<u8> = self.buffer.drain(..=pos).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let value = serde_json::from_slice::<serde_json::Value>(&line);
                if ready && !matches!(&value,Ok(v) if v.get("type").is_some()) {
                    self.pre_ready = self
                        .pre_ready
                        .checked_add(line.len() + 1)
                        .ok_or(SessionError::Capacity)?;
                    if self.pre_ready > 16384 {
                        return Err(SessionError::Capacity);
                    }
                    continue;
                }
                let v = value.map_err(|_| SessionError::Protocol)?;
                if v["v"].as_u64() != Some(1) || v["nonce"].as_str() != Some(&self.nonce) {
                    return Err(SessionError::Protocol);
                }
                if !self.buffer.is_empty() {
                    return Err(SessionError::Protocol);
                }
                return Ok(v);
            }
            let cap = if ready { 16384 } else { FRAME_CAP };
            if self.buffer.len() > cap {
                return Err(SessionError::Capacity);
            }
            let handle = self.handle.as_ref().ok_or(SessionError::Cancelled)?;
            let poll = handle.poll()?;
            for event in poll.events {
                match event {
                    super::session::OutputEvent::Bytes {
                        bytes, start, end, ..
                    } => {
                        if start != self.offset
                            || end
                                != start
                                    .checked_add(bytes.len() as u64)
                                    .ok_or(SessionError::Protocol)?
                        {
                            return Err(SessionError::Protocol);
                        }
                        self.offset = end;
                        self.buffer.extend_from_slice(&bytes);
                        if self.buffer.len() > FRAME_CAP + 16384 {
                            return Err(SessionError::Capacity);
                        }
                    }
                    super::session::OutputEvent::Gap { .. } => return Err(SessionError::Protocol),
                    _ => {}
                }
            }
            if poll.batch_id != 0 {
                handle.ack(poll.batch_id, 1)?;
            }
            if self.buffer.iter().any(|b| *b == b'\n') {
                continue;
            }
            if let Some(error) = poll.ended {
                return Err(error);
            }
            handle.wait_output().await;
        }
    }
    fn validate_reply(
        &self,
        v: serde_json::Value,
        request: &HistoryRequest,
        op: &str,
        epoch: Option<u64>,
        id: u64,
    ) -> Result<HistoryReply, SessionError> {
        if v["requestId"].as_u64() != Some(id) || v["instance"].as_str() != Some(&self.instance) {
            return Err(SessionError::Protocol);
        }
        match v["type"].as_str() {
            Some("error") => {
                let kind = v["kind"].as_str().ok_or(SessionError::Protocol)?;
                if !matches!(
                    kind,
                    "expired" | "unsupported" | "busy" | "limit" | "unavailable" | "protocol"
                ) {
                    return Err(SessionError::Protocol);
                }
                if kind == "unsupported" {
                    Ok(HistoryReply::Unsupported)
                } else {
                    Ok(HistoryReply::Error { error: kind.into() })
                }
            }
            Some("ack") => {
                if !matches!(op, "ack" | "keep" | "close")
                    || v["op"].as_str() != Some(op)
                    || v.get("epoch").and_then(|v| v.as_u64()) != epoch
                {
                    return Err(SessionError::Protocol);
                }
                Ok(HistoryReply::Ack {
                    op: op.into(),
                    instance: self.instance.clone(),
                    epoch,
                })
            }
            Some("page") => {
                if !matches!(op, "viewport" | "older") || v["op"].as_str() != Some(op) {
                    return Err(SessionError::Protocol);
                }
                let num = |field: &str| {
                    v[field]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or(SessionError::Protocol)
                };
                let e = v["epoch"]
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= SAFE_INT)
                    .ok_or(SessionError::Protocol)?;
                if epoch.is_some() && epoch != Some(e) {
                    return Err(SessionError::Protocol);
                }
                let columns = num("columns")?;
                let rows = num("rows")?;
                let row_count = num("rowCount")?;
                let start = num("start")?;
                let end = num("end")?;
                if columns == 0
                    || columns > 500
                    || rows == 0
                    || rows > 500
                    || row_count == 0
                    || row_count > 6000
                    || start > end
                    || end > row_count
                {
                    return Err(SessionError::Protocol);
                }
                let records: Vec<HistoryLine> = serde_json::from_value(v["records"].clone())
                    .map_err(|_| SessionError::Protocol)?;
                if records.len() != end - start
                    || records.len() > if op == "viewport" { rows } else { 256 }
                    || v["hasOlder"].as_bool() != Some(start > 0)
                {
                    return Err(SessionError::Protocol);
                }
                match request {
                    HistoryRequest::Viewport => {
                        if end != row_count || start < row_count.saturating_sub(rows) || start >= row_count {
                            return Err(SessionError::Protocol);
                        }
                    }
                    HistoryRequest::Older { before, limit, .. } => {
                        if end != *before || start != end.saturating_sub(*limit) {
                            return Err(SessionError::Protocol);
                        }
                    }
                    _ => return Err(SessionError::Protocol),
                }
                for (i, line) in records.iter().enumerate() {
                    if line.id != start + i + 1 {
                        return Err(SessionError::Protocol);
                    }
                    validate_line(line)?;
                }
                Ok(HistoryReply::Page {
                    instance: self.instance.clone(),
                    epoch: e,
                    columns,
                    rows,
                    row_count,
                    start,
                    end,
                    has_older: start > 0,
                    records,
                })
            }
            _ => Err(SessionError::Protocol),
        }
    }
}
fn validate_line(line: &HistoryLine) -> Result<(), SessionError> {
    let length = line.text.encode_utf16().count();
    if length > 8192 || line.styles.len() > 8192 {
        return Err(SessionError::Protocol);
    }
    let mut boundaries = std::collections::HashSet::new();
    let mut pos = 0usize;
    boundaries.insert(pos);
    for ch in line.text.chars() {
        pos += ch.len_utf16();
        boundaries.insert(pos);
    }
    let mut previous = 0;
    for &[start, end, fg, bg, flags] in &line.styles {
        if start < previous
            || start >= end
            || end > length as i64
            || !boundaries.contains(&(start as usize))
            || !boundaries.contains(&(end as usize))
            || !(-1..=0xffffff).contains(&fg)
            || !(-1..=0xffffff).contains(&bg)
            || !(0..=127).contains(&flags)
        {
            return Err(SessionError::Protocol);
        }
        previous = end;
    }
    Ok(())
}
#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
