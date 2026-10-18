"""Loopback official PTY wire fixture. Never executes terminal input."""
import asyncio
import base64
import copy
import datetime
import hashlib
import json
import struct
import ipaddress
import re
import secrets
import ssl
import threading
from aiohttp import web, WSMsgType
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID


class Fixture:
    def __init__(self, root):
        self.root = root
        self.credential = secrets.token_hex(32)
        self.sessions = {}
        self.events = []
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self.loop.run_forever, daemon=True)
        self.thread.start()
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
        server_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'art-term loopback fixture server')])
        server = (x509.CertificateBuilder().subject_name(server_name).issuer_name(name).public_key(server_key.public_key())
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
                                   'input': b'', 'previewInput': b'', 'bootstrap': False, 'controlNonce': None, 'resizes': [], 'attaches': [], 'ws': None}
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

    async def native_redraw(self, work):
        records = work.get('nativeRows', [])
        rows = work['resizes'][-1][0] if work['resizes'] else 24
        end = max(rows, len(records) - work.get('nativeOffset', 0))
        selected = records[max(0, end - rows):end]
        body = '\x1b[?1049h\x1b[?1000l\x1b[?1002l\x1b[?1006l'
        body += '\x1b[H\x1b[2J' + ''.join(f'\x1b[{i+1};1H\x1b[38;2;86;180;233m{text}\x1b[0m' for i, text in enumerate(selected))
        await self.output(work, (body + f'\x1b[{rows};1H').encode())

    async def ordinary_input(self, work, data):
        work['input'] += data
        await self.output(work, b''.join(b'\b \b' if value in (8, 127) else b'\r\n' if value == 13 else bytes([value]) for value in data))

    async def history_request(self, s):
        while b'\n' in s['previewInput']:
            raw, s['previewInput'] = s['previewInput'].split(b'\n', 1)
            request = json.loads(raw)
            if request.get('v') != 1 or request.get('nonce') != s['historyNonce']:
                raise ValueError('history request binding mismatch')
            op = request['op']
            reply = {'v': 1, 'nonce': s['historyNonce'], 'requestId': request['requestId'],
                     'instance': s['historyInstance'], 'op': op}
            event = {'method': 'history', 'name': s['historyWork'], 'action': op}
            if op in ('scroll', 'follow'):
                work = self.sessions.get(s['historyWork'])
                if work is None:
                    reply.update(type='error', kind='unavailable')
                elif work.get('nativeOwner') not in (None, s['historyInstance']):
                    reply.update(type='error', kind='busy')
                else:
                    work['nativeOwner'] = s['historyInstance']
                    work['nativeViewport'] = True
                    rows = work['resizes'][-1][0] if work['resizes'] else 24
                    records = work.get('nativeRows', [])
                    if op == 'scroll':
                        lines = request['lines']
                        if type(lines) is not int or not -256 <= lines <= 256:
                            raise ValueError('invalid native viewport lines')
                        work['nativeOffset'] = max(0, min(max(0, len(records)-rows), work.get('nativeOffset',0)-lines))
                    else:
                        work['nativeOffset'] = 0
                    await self.native_redraw(work)
                    if op == 'follow' and 'input' in request:
                        raw = base64.b64decode(request['input'], validate=True)
                        if len(raw)>4096 or base64.b64encode(raw).decode()!=request['input']:
                            raise ValueError('invalid follow input')
                        await self.ordinary_input(work,raw)
                    reply.update(type='view', held=work['nativeOffset']>0, offset=work['nativeOffset'], available=True)
                    event.update(lines=request.get('lines'), inputBytes=len(raw) if op=='follow' and 'input' in request else 0)
            elif op in ('viewport', 'older'):
                if op == 'viewport':
                    work = self.sessions.get(s['historyWork'])
                    if work is None:
                        reply.update(type='error', kind='unavailable')
                        await self.output(s, (json.dumps(reply) + '\n').encode())
                        continue
                    records = work.get('nativeRows') or work['history'].decode('utf-8').splitlines()[-5000:] or ['']
                    s['historyEpoch'] += 1
                    epoch = s['historyEpoch']
                    rows = work['resizes'][-1][0] if work['resizes'] else 24
                    columns = work['resizes'][-1][1] if work['resizes'] else 80
                    s['historySnapshots'][epoch] = (records, rows, columns)
                    for old in list(s['historySnapshots'])[:-2]:
                        del s['historySnapshots'][old]
                    end, limit = len(records), rows
                else:
                    epoch, end, limit = request['epoch'], request['before'], request['limit']
                    if not 1 <= limit <= 256 or epoch not in s['historySnapshots']:
                        raise ValueError('invalid fixture history page request')
                    records, rows, columns = s['historySnapshots'][epoch]
                start = max(0, end - limit)
                reply.update(type='page', epoch=epoch, columns=columns, rows=rows, rowCount=len(records),
                             start=start, end=end, hasOlder=start > 0,
                             records=[{'id': i + 1, 'text': records[i], 'styles': [[0, len(records[i].encode('utf-16-le')) // 2, 0x569cd6, -1, 0]] if records[i] else []} for i in range(start, end)])
                event.update(epoch=epoch, before=end, limit=limit)
            elif op in ('ack', 'keep', 'close'):
                if op == 'close':
                    work=self.sessions.get(s['historyWork'])
                    if work and work.get('nativeOwner')==s['historyInstance']:
                        work['nativeOwner']=None; work['nativeOffset']=0
                        await self.native_redraw(work)
                reply['type'] = 'ack'
                if op not in ('close',):
                    reply['epoch'] = request['epoch']
                    event['epoch'] = request['epoch']
            else:
                raise ValueError('unexpected history operation')
            self.events.append(event)
            await self.output(s, (json.dumps(reply) + '\n').encode())

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
                    if name.startswith('mcshot-'):
                        # Exec-only configure receiver: no command execution or secret retention.
                        s['previewInput'] += message.data
                        if 'historyNonce' in s:
                            await self.history_request(s)
                        elif s['controlNonce'] is None:
                            match = re.fullmatch(rb'exec python3 "\$HOME/\.local/share/art-term/tools/instance_bridge\.py" control --binding (th2-[a-f0-9]{24}) --nonce ([a-f0-9]{32})\n', s['previewInput'])
                            if match:
                                s['controlNonce'] = match[2].decode()
                                s['previewInput'] = b''
                                await self.output(s, ('ARTTERM_TOOLS_READY:' + s['controlNonce'] + '\r\n').encode())
                            else:
                                history = re.fullmatch(rb'exec "\$HOME/\.local/share/art-term/openab-pty/node" "\$HOME/\.local/share/art-term/openab-pty/(?:tmux-v2/)?tmux-history\.cjs" ([a-f0-9]{24}) ([a-f0-9]{32})\n', s['previewInput'])
                                if history:
                                    s.update(historyNonce=history[2].decode(), historyInstance=secrets.token_hex(16),
                                             historyWork='th2-' + history[1].decode(), historyEpoch=0, historySnapshots={})
                                    s['previewInput'] = b''
                                    await self.output(s, (json.dumps({'v': 1, 'type': 'ready', 'requestId': 0,
                                        'key': history[1].decode(), 'nonce': s['historyNonce'], 'instance': s['historyInstance'], 'nativeViewport': True}) + '\n').encode())
                        elif len(s['previewInput']) >= 4:
                            size = struct.unpack('!I', s['previewInput'][:4])[0]
                            if size > 16384:
                                raise ValueError('configure frame too large')
                            if len(s['previewInput']) == size + 4:
                                payload = json.loads(base64.b64decode(s['previewInput'][4:], validate=True))
                                if payload.get('op') not in ('configure', 'clear'):
                                    raise ValueError('unsupported configure operation')
                                self.events.append({'method': 'tools-control', 'name': name, 'action': payload['op']})
                                # No adapter/model exists. Explicitly report zero adapters.
                                answer = {'ok': True, 'adapters': 0, 'targets': []}
                                s['previewInput'] = b''
                                payload.clear()
                                await self.output(s, ('ARTTERM_TOOLS_RESULT:' + s['controlNonce'] + ':' + json.dumps(answer) + '\r\n').encode())
                    elif name.startswith('th2-') and not s['bootstrap']:
                        s['previewInput'] += message.data
                        if s['previewInput'].endswith(b'\n'):
                            key = name[4:].encode()
                            if not s['previewInput'].startswith(b'if [ -f ') or key not in s['previewInput'] or b'tmux-shell.py' not in s['previewInput']:
                                raise ValueError('unexpected history bootstrap')
                            s['bootstrap'] = True
                            s['previewInput'] = b''
                            self.events.append({'method': 'bootstrap', 'name': name})
                            await self.output(s, b'Fixed PTY fixture ready (no commands executed)\r\n')
                    else:
                        if not (s.get('nativeOffset',0) and message.data.startswith(b'\x1b')):
                            await self.ordinary_input(s, message.data)
                elif message.type == WSMsgType.TEXT:
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
        return {'events': copy.deepcopy(self.events), 'pngBytes': 0, 'sessions': {
            name: {'generation': s['generation'], 'input': s['input'].decode('utf-8'), 'bootstrap': s['bootstrap'],
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

    def finish(self, name):
        async def finish():
            ws = self.sessions[name]['ws']
            if ws is None:
                raise RuntimeError('fixture work socket absent')
            # Send the peer close frame before the handler can complete its
            # default close handshake (aiohttp concurrent close otherwise races).
            await ws._writer.close(code=4004, message=b'fixture finished')
        self.call(finish())

    def append_lines(self, name, start, count, wide=False):
        async def append():
            work=self.sessions[name]
            lines=[f'ROW-{i:04d}' + (' 繁體中文 English 自動換行' * 24 if wide and i%7==0 else '') for i in range(start,start+count)]
            work.setdefault('nativeRows',[]).extend(lines)
            if work.get('nativeViewport'):
                if work.get('nativeOffset',0): work['nativeOffset']+=count
                else: await self.native_redraw(work)
            else: await self.output(work, ('\r\n'.join(lines)+'\r\n').encode())
        self.call(append())

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
