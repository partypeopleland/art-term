"""Deterministic instance-mcp services; never connect to a model or real desktop."""
import base64
import json
import secrets
import struct
import threading
import time
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def png(color):
    def chunk(t, d):
        return struct.pack('!I', len(d))+t+d+struct.pack('!I', zlib.crc32(t+d) & 0xffffffff)
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('!IIBBBBB', 2, 2, 8, 2, 0, 0, 0))+chunk(b'IDAT', zlib.compress((b'\0'+bytes(color)*2)*2))+chunk(b'IEND', b'')


class Fixture:
    def __init__(self, name='A', sse=False, protocol='2025-03-26'):
        outer = self
        self.name, self.sse, self.protocol = name, sse, protocol
        self.token = secrets.token_hex(32)
        self.events, self.sessions = [], set()
        self.page = 'about:blank'
        self.delay = 0
        self.pause = False
        self.pause_list = False
        self.list_error = False
        self.list_started, self.list_release = threading.Event(), threading.Event()
        self.started, self.release = threading.Event(), threading.Event()
        self.image = png((240, 30, 20) if name == 'A' else (20, 30, 240))
        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'
            def log_message(self, *_): pass
            def reply(self, body, status=200, session=None):
                self.close_connection = True
                raw = json.dumps(body).encode()
                if outer.sse and status == 200:
                    raw = b'event: message\ndata: '+raw+b'\n\n'
                self.send_response(status)
                self.send_header('Content-Type', 'text/event-stream' if outer.sse else 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.send_header('Connection', 'close')
                if session: self.send_header('Mcp-Session-Id', session)
                self.end_headers()
                try: self.wfile.write(raw)
                except (BrokenPipeError, ConnectionResetError): pass  # expected client timeout
            def authorized(self):
                return self.headers.get('Authorization') == 'Bearer '+outer.token
            def do_DELETE(self):
                if not self.authorized(): return self.reply({}, 401)
                outer.sessions.discard(self.headers.get('Mcp-Session-Id'))
                self.reply({}, 200)
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if not self.authorized(): return self.reply({}, 401)
                outer.events.append({'method': body['method'], 'params': body.get('params', {})})
                method, params = body['method'], body.get('params', {})
                session = None
                if method == 'initialize':
                    session = secrets.token_hex(8); outer.sessions.add(session)
                    result = {'protocolVersion': outer.protocol, 'capabilities': {'tools': {}}, 'serverInfo': {'name': outer.name, 'version': '1'}}
                else:
                    if self.headers.get('Mcp-Session-Id') not in outer.sessions: return self.reply({}, 400)
                    if method == 'notifications/initialized': return self.reply({}, 202)
                    if method == 'tools/list':
                        if outer.list_error: return self.reply({'jsonrpc': '2.0', 'id': body.get('id'), 'error': {'code': -32000, 'message': 'fixture tools unavailable'}}, 500)
                        if outer.pause_list:
                            outer.pause_list = False; outer.list_started.set()
                            if not outer.list_release.wait(5): return self.reply({}, 504)
                        result = {'tools': [{'name': n, 'description': outer.name+' '+n, 'inputSchema': {'type': 'object'}} for n in ('browser_navigate', 'browser_title', 'screenshot', 'fail')]}
                    elif method == 'tools/call':
                        if outer.delay: time.sleep(outer.delay)
                        tool = params['name']
                        if outer.pause and tool == 'browser_navigate':
                            outer.pause = False; outer.started.set()
                            if not outer.release.wait(5): return self.reply({}, 504)
                        if tool == 'screenshot':
                            result = {'content': [{'type': 'image', 'mimeType': 'image/png', 'data': base64.b64encode(outer.image).decode()}]}
                        elif tool in ('browser_navigate', 'browser_title'):
                            if tool == 'browser_navigate': outer.page = params['arguments']['url']
                            result = {'content': [{'type': 'text', 'text': json.dumps({'desktop': outer.name, 'page': outer.page})}]}
                        elif tool == 'rich_content': result = {'isError': True, 'content': [{'type':'image','mimeType':'image/png','data':base64.b64encode(outer.image).decode()}, {'type':'audio','mimeType':'audio/wav','data':'QUJD'}, {'type':'resource','resource':{'uri':'fixture://resource','mimeType':'text/plain','text':'resource payload'}}], 'structuredContent':{'source':outer.name,'value':[1,2,3]}, '_meta':{'vendor':'untouched'}}
                        elif tool == 'token_echo': result = {'content':[{'type':'text','text':outer.token}]}
                        elif tool == 'fail': result = {'isError': True, 'content': [{'type': 'text', 'text': 'fixture failure'}]}
                        else: return self.reply({'jsonrpc': '2.0', 'id': body['id'], 'error': {'code': -32601}}, 200)
                    else: return self.reply({}, 400)
                self.reply({'jsonrpc': '2.0', 'id': body.get('id'), 'result': result}, session=session)
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True); self.thread.start()
        self.url = f'http://127.0.0.1:{self.server.server_port}/mcp'
    def config(self):
        return {'op': 'configure', 'id': 'desktop-'+self.name, 'revision': 1, 'name': self.name, 'url': self.url, 'token': self.token}
    def close(self): self.server.shutdown(); self.server.server_close(); self.thread.join(timeout=5)
