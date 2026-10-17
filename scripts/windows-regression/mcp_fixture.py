"""Loopback MCP screenshot upstream. Fixed pixels; no model, desktop, or AI CLI."""
import base64
import json
import secrets
import struct
import threading
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def png(color):
    def chunk(kind, data):
        return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!IIBBBBB', 2, 2, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress((b'\0' + bytes(color) * 2) * 2)) + chunk(b'IEND', b''))


class McpFixture:
    def __init__(self, name, sse=False):
        outer = self
        self.name, self.sse = name, sse
        self.token = secrets.token_hex(32)
        self.image = png((240, 30, 20) if name == 'A' else (20, 30, 240))
        self.events, self.sessions = [], set()

        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'
            def log_message(self, *_): pass
            def reply(self, value, status=200, session=None):
                raw = json.dumps(value).encode()
                if sse and status == 200:
                    raw = b'event: message\ndata: ' + raw + b'\n\n'
                self.send_response(status)
                self.send_header('Content-Type', 'text/event-stream' if sse else 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.send_header('Connection', 'close')
                if session: self.send_header('Mcp-Session-Id', session)
                self.end_headers()
                self.wfile.write(raw)
                self.close_connection = True
            def authorized(self):
                return self.path == '/mcp' and self.headers.get('Authorization') == 'Bearer ' + outer.token
            def do_DELETE(self):
                if not self.authorized(): return self.reply({}, 401)
                outer.sessions.discard(self.headers.get('Mcp-Session-Id'))
                outer.events.append({'method': 'DELETE'})
                self.reply({})
            def do_POST(self):
                if not self.authorized(): return self.reply({}, 401)
                size = int(self.headers.get('Content-Length', '0'))
                if not 0 < size <= 65536: return self.reply({}, 400)
                body = json.loads(self.rfile.read(size))
                method = body.get('method')
                outer.events.append({'method': method})
                session = None
                if method == 'initialize':
                    session = secrets.token_hex(16)
                    outer.sessions.add(session)
                    result = {'protocolVersion': '2025-03-26', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'fixed-' + name, 'version': '1'}}
                else:
                    if self.headers.get('Mcp-Session-Id') not in outer.sessions: return self.reply({}, 400)
                    if method == 'notifications/initialized': return self.reply({}, 202)
                    if method != 'tools/call' or body.get('params', {}).get('name') != 'screenshot': return self.reply({}, 400)
                    result = {'content': [{'type': 'image', 'mimeType': 'image/png', 'data': base64.b64encode(outer.image).decode()}]}
                self.reply({'jsonrpc': '2.0', 'id': body.get('id'), 'result': result}, session=session)
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f'http://127.0.0.1:{self.server.server_port}/mcp'

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
