#!/usr/bin/python3
"""Observation-only MCP desktop bridge. Trusted image/env inputs; no host access."""
import base64
import json
import os
import sys
import urllib.parse
import urllib.request

MAX_REPLY = 16 * 1024 * 1024
MAX_IMAGE = 8 * 1024 * 1024


class RefuseRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def require(ok):
    if not ok:
        raise ValueError("invalid desktop response")


class Mcp:
    def __init__(self):
        self.url = os.environ["ART_TERM_DESKTOP_MCP_URL"]
        parsed = urllib.parse.urlsplit(self.url)
        require(parsed.scheme in ("http", "https") and parsed.hostname
                and not parsed.username and not parsed.password
                and not parsed.query and not parsed.fragment
                and parsed.path == "/mcp" and len(self.url) <= 2048)
        self.token = os.environ["HANDS_MCP_TOKEN"]
        require(0 < len(self.token) <= 4096 and self.token.isascii()
                and not any(ord(c) < 33 or ord(c) > 126 for c in self.token))
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}), RefuseRedirect())
        self.session = None
        self.version = "2025-03-26"
        self.counter = 0
        result = self.call("initialize", {
            "protocolVersion": self.version, "capabilities": {},
            "clientInfo": {"name": "hana-desktop-preview", "version": "1"}})
        # The server may negotiate an older protocol supported by this bridge.
        negotiated = result.get("protocolVersion")
        require(negotiated in ("2024-11-05", "2025-03-26"))
        self.version = negotiated
        self.call("notifications/initialized", None, notification=True)

    def call(self, method, params, notification=False):
        self.counter += 1
        payload = {"jsonrpc": "2.0", "method": method}
        if not notification:
            payload["id"] = self.counter
        if params is not None:
            payload["params"] = params
        headers = {"Authorization": "Bearer " + self.token,
                   "Content-Type": "application/json",
                   "Accept": "application/json, text/event-stream",
                   "MCP-Protocol-Version": self.version}
        if self.session is not None:
            headers["Mcp-Session-Id"] = self.session
        request = urllib.request.Request(self.url, json.dumps(payload).encode(), headers)
        with self.opener.open(request, timeout=5) as response:
            length = response.headers.get("Content-Length")
            require(length is None or 0 <= int(length) <= MAX_REPLY)
            data = response.read(MAX_REPLY + 1)
            require(len(data) <= MAX_REPLY)
            sid = response.headers.get("Mcp-Session-Id")
            if sid is not None:
                require(0 < len(sid) <= 256 and sid.isascii()
                        and all(33 <= ord(c) <= 126 for c in sid))
                require(self.session is None or self.session == sid)
                self.session = sid
            content_type = response.headers.get_content_type()
        if notification:
            require(not data or len(data) <= 4096)
            return {}
        if content_type == "application/json":
            replies = [json.loads(data)]
        else:
            require(content_type == "text/event-stream")
            replies, parts = [], []
            # Strict bounded SSE frames; comments and event/id lines carry no RPC data.
            for line in data.decode("utf-8").replace("\r\n", "\n").split("\n") + [""]:
                if line == "":
                    if parts:
                        replies.append(json.loads("\n".join(parts)))
                        parts = []
                elif line.startswith("data:"):
                    parts.append(line[5:].lstrip(" "))
                else:
                    require(line.startswith((":", "event:", "id:", "retry:")))
        matching = [v for v in replies if isinstance(v, dict)
                    and v.get("id") == self.counter]
        require(len(matching) == 1)
        reply = matching[0]
        require(reply.get("jsonrpc") == "2.0" and "error" not in reply
                and isinstance(reply.get("result"), dict))
        return reply["result"]


def main():
    require(len(sys.argv) == 2 and sys.argv[1] in ("status", "screenshot"))
    client = Mcp()
    # instance-mcp's local desktop tool; listing all tools also queries its browser
    # upstream and can block on browser startup/cache refresh. Never depend on it.
    result = client.call("tools/call", {"name": "screenshot", "arguments": {}})
    require(result.get("isError") is not True)
    content = result.get("content")
    require(isinstance(content, list))
    images = [item for item in content if isinstance(item, dict) and item.get("type") == "image"]
    require(len(images) == 1)
    image = images[0]
    mime, encoded = image.get("mimeType"), image.get("data")
    require(mime in ("image/png", "image/jpeg") and isinstance(encoded, str)
            and 0 < len(encoded) <= 4 * ((MAX_IMAGE + 2) // 3))
    raw = base64.b64decode(encoded, validate=True)
    require(0 < len(raw) <= MAX_IMAGE)
    require((mime == "image/png" and raw.startswith(b"\x89PNG\r\n\x1a\n"))
            or (mime == "image/jpeg" and raw.startswith(b"\xff\xd8\xff")))
    if sys.argv[1] == "status":
        return {"enabled": True}
    return {"enabled": True, "mime": mime, "image_base64": encoded, "bytes": len(raw)}


if __name__ == "__main__":
    try:
        output = main()
    except Exception:
        # Do not echo URL, token, tool errors, payloads, or exceptions.
        sys.stdout.write('{"error":"desktop MCP unavailable"}\n')
        sys.exit(1)
    sys.stdout.write(json.dumps(output, separators=(",", ":")) + "\n")
