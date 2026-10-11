"""Loopback official PTY wire fixture. Never executes terminal input."""
import asyncio
import base64
import copy
import datetime
import hashlib
import io
import ipaddress
import random
import re
import secrets
import ssl
import threading
from aiohttp import web, WSMsgType
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID
from PIL import Image


class Fixture:
    def __init__(self, root):
        self.root = root
        self.credential = secrets.token_hex(32)
        self.sessions = {}
        self.events = []
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self.loop.run_forever, daemon=True)
        self.thread.start()
        image = Image.frombytes('RGB', (512, 256), random.Random(47).randbytes(512 * 256 * 3))
        out = io.BytesIO()
        image.save(out, format='PNG', compress_level=0)
        self.png = out.getvalue()
        assert len(self.png) > 256 * 1024
        self.ca = root / 'fixture-ca.der'
        self.context = self._certificates()
        self.call(self._start())

    def call(self, coroutine):
        return asyncio.run_coroutine_threadsafe(coroutine, self.loop).result(timeout=20)

    def _certificates(self):
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'art-term ephemeral fixture')])
        now = datetime.datetime.now(datetime.timezone.utc)
        ca = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key())
              .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(minutes=1))
              .not_valid_after(now + datetime.timedelta(days=1))
              .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
              .sign(key, hashes.SHA256()))
        server_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        server = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(server_key.public_key())
                  .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(minutes=1))
                  .not_valid_after(now + datetime.timedelta(days=1))
                  .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), False)
                  .sign(key, hashes.SHA256()))
        self.ca.write_bytes(ca.public_bytes(serialization.Encoding.DER))
        cert_path, key_path = self.root / 'server.pem', self.root / 'server-key.pem'
        cert_path.write_bytes(server.public_bytes(serialization.Encoding.PEM))
        key_path.write_bytes(server_key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert_path, key_path)
        return context

    async def _start(self):
        app = web.Application(client_max_size=65536)
        app.router.add_route('*', '/admin/sessions', self.admin)
        app.router.add_route('*', '/admin/sessions/{name}/{action}', self.admin)
        app.router.add_route('*', '/admin/sessions/{name}', self.admin)
        app.router.add_get('/pty/{name}', self.pty)
        self.runner = web.AppRunner(app, access_log=None)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, '127.0.0.1', 0, ssl_context=self.context)
        await self.site.start()
        self.origin = 'https://127.0.0.1:' + str(self.site._server.sockets[0].getsockname()[1])

    def grant(self, name):
        s = self.sessions[name]
        return {'session': name, 'generation': s['generation'], 'token': s['token'], 'token_expires_in_secs': 3600}

    async def admin(self, request):
        if request.headers.get('Authorization') != 'Bearer ' + self.credential:
            return web.json_response({}, status=401)
        name, action = request.match_info.get('name'), request.match_info.get('action')
        self.events.append({'method': request.method, 'name': name, 'action': action})
        if request.method == 'GET' and name is None:
            return web.json_response({'sessions': [
                {'name': n, 'generation': s['generation'], 'alive': True,
                 'attached': s['ws'] is not None and not s['ws'].closed, 'bytes_written': len(s['history']),
                 'tier': 'tier1', 'teardown_best_effort': True, 'absolute_ttl_best_effort': True}
                for n, s in self.sessions.items()], 'draining': False})
        if request.method == 'POST' and name is None:
            body = await request.json()
            name = body.get('name', '')
            if set(body) != {'name'} or not re.fullmatch('[a-z0-9-]{1,32}', name):
                return web.json_response({}, status=400)
            self.events[-1]['name'] = name
            if name in self.sessions:
                return web.json_response({}, status=409)
            self.sessions[name] = {'generation': 1, 'token': secrets.token_hex(32), 'history': b'',
                                   'input': b'', 'previewInput': b'', 'resizes': [], 'attaches': [], 'ws': None}
            return web.json_response(self.grant(name))
        if name not in self.sessions:
            return web.json_response({}, status=404)
        s = self.sessions[name]
        if request.method == 'POST' and action == 'renew':
            if await request.read():
                return web.json_response({}, status=400)
            if s['ws'] is not None:
                await s['ws'].close(code=4001)
            s['generation'] += 1
            s['token'] = secrets.token_hex(32)
            return web.json_response(self.grant(name))
        if request.method == 'DELETE' and action is None:
            if s['ws'] is not None:
                await s['ws'].close()
            del self.sessions[name]
            return web.json_response({})
        return web.json_response({}, status=400)

    async def output(self, s, data):
        s['history'] += data
        for pos in range(0, len(data), 32768):
            await s['ws'].send_bytes(data[pos:pos + 32768])
            await asyncio.sleep(0)

    async def pty(self, request):
        name = request.match_info['name']
        s = self.sessions.get(name)
        if not s or request.headers.get('Authorization') != 'Bearer ' + s['token']:
            return web.Response(status=401)
        since = int(request.query.get('since', '0'))
        if since > len(s['history']):
            return web.Response(status=400)
        ws = web.WebSocketResponse(max_msg_size=262144, autoping=True)
        await ws.prepare(request)
        s['ws'] = ws
        s['attaches'].append({'since': since, 'tokenHash': hashlib.sha256(s['token'].encode()).hexdigest()})
        await ws.send_json({'v': 1, 'type': 'attach-notice', 'stream_offset': len(s['history']),
                            'ephemeral_workspace': True, 'teardown_best_effort': True})
        if s['history'][since:]:
            await ws.send_bytes(s['history'][since:])
        try:
            async for message in ws:
                if message.type == WSMsgType.BINARY:
                    s['input'] += message.data
                    if name.startswith('mcshot-'):
                        s['previewInput'] += message.data
                        match = re.fullmatch(rb'python3 "\$HOME/\.local/share/art-term/openab-pty/preview\.py" (status|screenshot) ([a-f0-9]{32})\n', s['previewInput'])
                        if match:
                            import json
                            action, nonce = match[1].decode(), match[2].decode()
                            value = {'enabled': True} if action == 'status' else {
                                'mime': 'image/png', 'image_base64': base64.b64encode(self.png).decode(), 'bytes': len(self.png)}
                            self.events.append({'method': 'preview', 'name': name, 'action': action})
                            data = ('ARTTERM_PREVIEW_BEGIN:' + nonce + '\r\n' + json.dumps(value, separators=(',', ':')) +
                                    '\r\nARTTERM_PREVIEW_END:' + nonce + ':0\r\n').encode()
                            await self.output(s, data)
                    else:
                        await self.output(s, message.data.replace(b'\r', b'\r\n'))
                elif message.type == WSMsgType.TEXT:
                    import json
                    control = json.loads(message.data)
                    if control.get('v') != 1:
                        raise ValueError('control version')
                    if control['type'] == 'resize':
                        s['resizes'].append([control['rows'], control['cols']])
                    elif control['type'] == 'ping':
                        await ws.send_json({'v': 1, 'type': 'pong'})
                    elif control['type'] == 'detach':
                        self.events.append({'method': 'detach', 'name': name})
                        break
                    else:
                        raise ValueError('unexpected control')
        finally:
            if s['ws'] is ws:
                s['ws'] = None
        return ws

    async def _snapshot(self):
        return {'events': copy.deepcopy(self.events), 'pngBytes': len(self.png), 'sessions': {
            name: {'generation': s['generation'], 'input': s['input'].decode('utf-8'),
                   'historyBytes': len(s['history']), 'resizes': copy.deepcopy(s['resizes']),
                   'attaches': copy.deepcopy(s['attaches']), 'attached': s['ws'] is not None}
            for name, s in self.sessions.items()}}

    def snapshot(self):
        return self.call(self._snapshot())

    async def _disconnect(self, name):
        ws = self.sessions[name]['ws']
        if ws is None:
            raise RuntimeError('fixture work socket absent')
        ws._req.transport.abort()

    def disconnect(self, name):
        self.call(self._disconnect(name))

    def close(self):
        async def stop():
            for s in list(self.sessions.values()):
                if s['ws'] is not None:
                    await s['ws'].close()
            await self.runner.cleanup()
        try:
            self.call(stop())
        finally:
            self.loop.call_soon_threadsafe(self.loop.stop)
            self.thread.join(timeout=5)
            if self.thread.is_alive():
                raise RuntimeError('fixture cleanup timeout')
            self.loop.close()
