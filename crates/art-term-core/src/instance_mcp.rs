//! Native MCP operations; credentials never enter frontend readback DTOs.
use crate::openab::{api::ApiError, secret::Secret};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

pub const LIMIT: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopProfile {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub mcp_url: String,
    pub token_present: bool,
}
pub struct DesktopSnapshot {
    pub profile: DesktopProfile,
    pub token: Secret,
}
pub fn canonical_url(raw: &str) -> Result<String, ApiError> {
    if raw.len() > 2048
        || raw.trim() != raw
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
    {
        return Err(ApiError::Protocol);
    }
    if raw
        .split_once("://")
        .is_none_or(|(_, rest)| rest.split('/').next().unwrap_or_default().contains('@'))
    {
        return Err(ApiError::Protocol);
    }
    let u = url::Url::parse(raw).map_err(|_| ApiError::Protocol)?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || u.path() != "/mcp"
    {
        return Err(ApiError::Protocol);
    }
    Ok(u.to_string())
}
fn trusted_http(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(ip) => u32::from(ip) & 0xffc00000 == 0x64400000,
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}
pub struct Mcp {
    http: reqwest::Client,
    url: String,
    token: Secret,
    session: Option<String>,
    protocol: String,
    next: u64,
    deadline: tokio::time::Instant,
}
impl Mcp {
    pub async fn connect(snapshot: DesktopSnapshot) -> anyhow::Result<Self> {
        anyhow::ensure!(snapshot.token.expose().is_ascii(), "桌面 token 格式不符");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let url = canonical_url(&snapshot.profile.mcp_url)
            .map_err(|_| anyhow::anyhow!("桌面網址格式不符"))?;
        let u = url::Url::parse(&url)?;
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20));
        if u.scheme() == "http" {
            let host = u.host_str().unwrap().trim_matches(['[', ']']);
            let addrs: Vec<SocketAddr> = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::lookup_host((host, u.port_or_known_default().unwrap())),
            )
            .await??
            .collect();
            anyhow::ensure!(
                !addrs.is_empty() && addrs.iter().all(|a| trusted_http(a.ip())),
                "HTTP 桌面須位於 Tailnet 或本機；其他網址請用 HTTPS"
            );
            builder = builder.resolve_to_addrs(host, &addrs);
        }
        let mut m = Self {
            http: builder.build()?,
            url,
            token: snapshot.token,
            session: None,
            protocol: "2025-03-26".into(),
            next: 0,
            deadline,
        };
        let initialized = m.call("initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name":"art-term","version":"1"}})).await;
        let result = async {
            let result = initialized?;
            let version = result["protocolVersion"].as_str().unwrap_or_default();
            anyhow::ensure!(
                ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"].contains(&version),
                "不支援的 MCP 協定"
            );
            m.protocol = version.into();
            m.request(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                None,
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = result {
            m.close().await;
            return Err(e);
        }
        Ok(m)
    }
    async fn request(&mut self, body: Value, id: Option<u64>) -> anyhow::Result<Value> {
        let mut req = self
            .http
            .post(&self.url)
            .bearer_auth(self.token.expose())
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", &self.protocol)
            .json(&body)
            .timeout(
                self.deadline
                    .saturating_duration_since(tokio::time::Instant::now()),
            );
        if let Some(s) = &self.session {
            req = req.header("Mcp-Session-Id", s);
        }
        let res = req.send().await?;
        anyhow::ensure!(res.status().is_success(), "MCP 認證或服務請求失敗");
        if let Some(s) = res.headers().get("Mcp-Session-Id") {
            let s = s.to_str()?;
            anyhow::ensure!(
                s.len() <= 1024 && !s.is_empty() && s.bytes().all(|b| (33..=126).contains(&b)),
                "MCP session 格式不符"
            );
            self.session = Some(s.into());
        }
        if id.is_none() && res.status() == reqwest::StatusCode::ACCEPTED {
            return Ok(Value::Null);
        }
        let content_type = res
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        anyhow::ensure!(
            id.is_none() || matches!(content_type, "application/json" | "text/event-stream"),
            "MCP 回應類型不符"
        );
        let sse = content_type == "text/event-stream";
        let mut bytes = Vec::new();
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            anyhow::ensure!(bytes.len() + chunk.len() <= LIMIT, "MCP 回應過大");
            bytes.extend_from_slice(&chunk);
            if sse {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    let normalized = text.replace("\r\n", "\n");
                    let complete = normalized
                        .rfind("\n\n")
                        .map(|n| &normalized[..n + 2])
                        .unwrap_or("");
                    for event in complete.split("\n\n") {
                        let data = event
                            .lines()
                            .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if let Ok(r) = serde_json::from_str::<Value>(&data) {
                            if id.is_some() && r.get("id").and_then(Value::as_u64) == id {
                                anyhow::ensure!(
                                    r["jsonrpc"] == "2.0" && r.get("error").is_none(),
                                    "MCP 回應失敗"
                                );
                                return r
                                    .get("result")
                                    .cloned()
                                    .ok_or_else(|| anyhow::anyhow!("MCP 缺少結果"));
                            }
                        }
                    }
                }
            }
        }
        if id.is_none() {
            return Ok(Value::Null);
        }
        let id = id.unwrap();
        let replies: Vec<Value> = if sse {
            std::str::from_utf8(&bytes)?
                .replace("\r\n", "\n")
                .split("\n\n")
                .filter_map(|e| {
                    let data = e
                        .lines()
                        .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                        .collect::<Vec<_>>()
                        .join("\n");
                    serde_json::from_str::<Value>(&data).ok()
                })
                .collect()
        } else {
            vec![serde_json::from_slice(&bytes)?]
        };
        let mut matching = replies.into_iter().filter(|r| r["id"] == id);
        let r = matching
            .next()
            .ok_or_else(|| anyhow::anyhow!("MCP 缺少回應"))?;
        anyhow::ensure!(
            matching.next().is_none() && r["jsonrpc"] == "2.0" && r.get("error").is_none(),
            "MCP 回應格式不符"
        );
        r.get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("MCP 缺少結果"))
    }
    pub async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next += 1;
        self.request(
            json!({"jsonrpc":"2.0", "id":self.next, "method":method,"params":params}),
            Some(self.next),
        )
        .await
    }
    pub async fn close(&mut self) {
        if let Some(s) = self.session.take() {
            let _ = self
                .http
                .delete(&self.url)
                .bearer_auth(self.token.expose())
                .header("Mcp-Session-Id", s)
                .header("MCP-Protocol-Version", &self.protocol)
                .timeout(Duration::from_secs(2))
                .send()
                .await;
        }
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let http = self.http.clone();
        let url = self.url.clone();
        let protocol = self.protocol.clone();
        let Ok(token) = Secret::new(self.token.expose().to_owned()) else {
            return;
        };
        runtime.spawn(async move {
            let _ = http
                .delete(url)
                .bearer_auth(token.expose())
                .header("Mcp-Session-Id", session)
                .header("MCP-Protocol-Version", protocol)
                .timeout(Duration::from_secs(2))
                .send()
                .await;
        });
    }
}
pub async fn screenshot(
    snapshot: DesktopSnapshot,
) -> anyhow::Result<crate::openab::screenshot::ShotImage> {
    use base64::Engine;
    let mut m = Mcp::connect(snapshot).await?;
    let response = m
        .call("tools/call", json!({"name":"screenshot", "arguments":{}}))
        .await;
    m.close().await;
    let r = response?;
    anyhow::ensure!(r["isError"] != true, "桌面截圖失敗");
    let images: Vec<_> = r["content"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("缺少截圖"))?
        .iter()
        .filter(|v| v["type"] == "image")
        .collect();
    anyhow::ensure!(images.len() == 1, "截圖格式不符");
    let image = images[0];
    let data = image["data"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("缺少圖片"))?;
    anyhow::ensure!(data.len() <= 12 * 1024 * 1024, "圖片過大");
    let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    Ok(crate::openab::screenshot::decode_screenshot(
        json!({"enabled":true,"mime":image["mimeType"],"image_base64":data,"bytes":bytes.len()}),
    )
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn snapshot(url: String) -> DesktopSnapshot {
        DesktopSnapshot {
            profile: DesktopProfile {
                id: "fixture-desktop".into(),
                revision: 1,
                name: "Fixture".into(),
                mcp_url: url,
                token_present: true,
            },
            token: Secret::new("fixture-native-token".into()).unwrap(),
        }
    }
    fn image() -> String {
        use image::ImageEncoder;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[25, 50, 75], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }
    async fn fixture(
        sse: bool,
        bad_id: bool,
        is_error: bool,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut methods = Vec::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut byte = [0u8; 1];
                while !raw.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).await.unwrap();
                    raw.push(byte[0]);
                }
                let header = String::from_utf8(raw).unwrap();
                assert!(header
                    .to_lowercase()
                    .contains("authorization: bearer fixture-native-token"));
                if header.starts_with("DELETE") {
                    methods.push("DELETE".into());
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                    return methods;
                }
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).await.unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                let method = request["method"].as_str().unwrap();
                methods.push(method.into());
                if method == "notifications/initialized" {
                    assert!(header
                        .to_lowercase()
                        .contains("mcp-session-id: native-fixture-session"));
                    stream.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    continue;
                }
                let result = if method == "initialize" {
                    json!({"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}})
                } else {
                    assert_eq!(method, "tools/call");
                    assert_eq!(request["params"]["name"], "screenshot");
                    json!({"isError":is_error,"content":[{"type":"image","mimeType":"image/png","data":image()}]})
                };
                let reply = json!({"jsonrpc":"2.0","id":if bad_id && method != "initialize" {json!(999)} else {request["id"].clone()},"result":result});
                let body = if sse {
                    format!("event: message\r\ndata: {reply}\r\n\r\n")
                } else {
                    reply.to_string()
                };
                let content_type = if sse {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nMcp-Session-Id: native-fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (url, task)
    }
    #[tokio::test]
    async fn direct_screenshot_negotiates_json_and_sse_without_tools_list() {
        for sse in [false, true] {
            let (url, task) = fixture(sse, false, false).await;
            let shot = screenshot(snapshot(url)).await.unwrap();
            assert_eq!(shot.mime, "image/png");
            assert_eq!(
                task.await.unwrap(),
                [
                    "initialize",
                    "notifications/initialized",
                    "tools/call",
                    "DELETE"
                ]
            );
        }
    }
    #[tokio::test]
    async fn direct_screenshot_rejects_wrong_rpc_identity_and_tool_error_and_deletes_session() {
        for (bad_id, is_error) in [(true, false), (false, true)] {
            let (url, task) = fixture(true, bad_id, is_error).await;
            assert!(screenshot(snapshot(url)).await.is_err());
            assert_eq!(task.await.unwrap().last().unwrap(), "DELETE");
        }
    }
    #[tokio::test]
    async fn direct_http_is_limited_to_tailnet_or_loopback() {
        assert!(Mcp::connect(snapshot("http://192.168.1.1/mcp".into()))
            .await
            .is_err());
        for ip in [
            "127.0.0.1",
            "::1",
            "100.64.0.1",
            "100.127.255.255",
            "fd7a:115c:a1e0::1",
        ] {
            assert!(trusted_http(ip.parse().unwrap()));
        }
        for ip in [
            "192.168.1.1",
            "10.0.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "fd7a:115c:a1e1::1",
        ] {
            assert!(!trusted_http(ip.parse().unwrap()));
        }
        for url in [
            "http://user@127.0.0.1/mcp",
            "http://@127.0.0.1/mcp",
            "http://127.0.0.1/mcp?token=x",
            "http://127.0.0.1/mcp#fragment",
            "ftp://127.0.0.1/mcp",
        ] {
            assert!(canonical_url(url).is_err());
        }
    }
}
