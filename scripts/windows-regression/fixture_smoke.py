"""Linux-safe protocol smoke only; never launches a Windows EXE or mocks IPC."""
import asyncio
import base64
import io
import json
from pathlib import Path
import ssl
import struct
import tempfile
from urllib.request import Request, urlopen
import aiohttp
from PIL import Image
from fixture import Fixture
from mcp_fixture import McpFixture


def rpc(upstream, method, params=None, session=None, identifier=1):
    headers = {'Authorization': 'Bearer ' + upstream.token, 'Content-Type': 'application/json'}
    if session: headers['Mcp-Session-Id'] = session
    body = {'jsonrpc': '2.0', 'method': method, 'params': params or {}}
    if identifier is not None: body['id'] = identifier
    with urlopen(Request(upstream.url, data=json.dumps(body).encode(), headers=headers), timeout=5) as response:
        raw = response.read()
        if upstream.sse and response.status == 200:
            raw = raw.split(b'data: ', 1)[1].strip()
        return response.headers.get('Mcp-Session-Id'), json.loads(raw)


def screenshots():
    for sse, name in [(False, 'A'), (True, 'B')]:
        upstream = McpFixture(name, sse)
        try:
            session, ready = rpc(upstream, 'initialize')
            assert ready['result']['protocolVersion'] == '2025-03-26'
            rpc(upstream, 'notifications/initialized', session=session, identifier=None)
            _, shot = rpc(upstream, 'tools/call', {'name': 'screenshot', 'arguments': {}}, session=session)
            image = base64.b64decode(shot['result']['content'][0]['data'], validate=True)
            with Image.open(io.BytesIO(image)) as decoded:
                decoded.load()
                assert decoded.size == (2, 2) and decoded.format == 'PNG'
            with urlopen(Request(upstream.url, method='DELETE', headers={'Authorization': 'Bearer ' + upstream.token, 'Mcp-Session-Id': session}), timeout=5) as response:
                assert response.status == 200
            assert not upstream.sessions
        finally:
            upstream.close()
            assert not upstream.thread.is_alive() and upstream.server.fileno() == -1


async def pty(fixture):
    context = ssl.create_default_context(cadata=ssl.DER_cert_to_PEM_cert(fixture.ca.read_bytes()))
    async with aiohttp.ClientSession() as http:
        async def create(name):
            async with http.post(fixture.origin + '/admin/sessions', ssl=context,
                                 headers={'Authorization': 'Bearer ' + fixture.credential}, json={'name': name}) as response:
                assert response.status == 200
                return await response.json()
        name = 'th2-' + 'a' * 24
        grant = await create(name)
        url = fixture.origin.replace('https:', 'wss:') + '/pty/' + name
        async with http.ws_connect(url, ssl=context, headers={'Authorization': 'Bearer ' + grant['token']}) as ws:
            await ws.receive_json(timeout=3)
            bootstrap = ('if [ -f "$HOME/.local/share/art-term/tools/tmux-shell.py" ]; then exec python3 "$HOME/.local/share/art-term/tools/tmux-shell.py" ' + name[4:] + '; fi\n').encode()
            await ws.send_bytes(bootstrap[:17]); await ws.send_bytes(bootstrap[17:])
            assert b'fixture ready' in (await ws.receive(timeout=3)).data
            await ws.send_bytes('繁體測試\r'.encode())
            assert (await ws.receive(timeout=3)).data == '繁體測試\r\n'.encode()
            await ws.send_json({'v': 1, 'type': 'resize', 'rows': 30, 'cols': 90})
        for index, operation in enumerate(('configure', 'clear')):
            temporary = 'mcshot-' + str(index)
            grant = await create(temporary)
            url = fixture.origin.replace('https:', 'wss:') + '/pty/' + temporary
            async with http.ws_connect(url, ssl=context, headers={'Authorization': 'Bearer ' + grant['token']}) as ws:
                await ws.receive_json(timeout=3)
                nonce = 'b' * 32
                command = f'exec python3 "$HOME/.local/share/art-term/tools/instance_bridge.py" control --binding {name} --nonce {nonce}\n'.encode()
                await ws.send_bytes(command[:13]); await ws.send_bytes(command[13:])
                assert (await ws.receive(timeout=3)).data == f'ARTTERM_TOOLS_READY:{nonce}\r\n'.encode()
                encoded = base64.b64encode(json.dumps({'op': operation, 'token': 'never-retain-this-token'}).encode())
                frame = struct.pack('!I', len(encoded)) + encoded
                await ws.send_bytes(frame[:7]); await ws.send_bytes(frame[7:])
                answer = (await ws.receive(timeout=3)).data
                assert f'ARTTERM_TOOLS_RESULT:{nonce}:'.encode() in answer
                assert b'"adapters": 0' in answer
            async with http.delete(fixture.origin + '/admin/sessions/' + temporary, ssl=context,
                                   headers={'Authorization': 'Bearer ' + fixture.credential}) as response:
                assert response.status == 200
        state = fixture.snapshot()
        assert state['sessions'][name]['bootstrap']
        assert state['sessions'][name]['resizes'] == [[30, 90]]
        assert 'never-retain-this-token' not in json.dumps(state)
        assert sum(e['method'] == 'tools-control' for e in state['events']) == 2
        # Code 4004 must remain an explicit terminal-exit event.
        grant = fixture.grant(name)
        async with http.ws_connect(fixture.origin.replace('https:', 'wss:') + '/pty/' + name, ssl=context,
                                   headers={'Authorization': 'Bearer ' + grant['token']}) as ws:
            await ws.receive_json(timeout=3)
            await ws.receive(timeout=3)  # replay retained output
            finished = asyncio.create_task(asyncio.to_thread(fixture.finish, name))
            await ws.receive(timeout=3)
            await finished
            assert ws.close_code == 4004, repr((ws.close_code, ws.exception()))


async def viewport(fixture):
    class Socket:
        def __init__(self): self.frames = []
        async def send_bytes(self, data): self.frames.append(data)
    work = {'history': b'', 'input': b'', 'resizes': [[3, 80]], 'nativeRows': [f'ROW-{i:04}' for i in range(1100)], 'ws': Socket()}
    helper = {'history': b'', 'previewInput': b'', 'historyNonce': 'smoke', 'historyInstance': 'owner', 'historyWork': 'viewport-smoke', 'historyEpoch': 0, 'historySnapshots': {}, 'ws': Socket()}
    fixture.sessions['viewport-smoke'] = work
    async def request(op, **fields):
        helper['previewInput'] += (json.dumps({'v': 1, 'nonce': 'smoke', 'requestId': 1, 'op': op, **fields})+'\n').encode()
        await fixture.history_request(helper)
        return json.loads(helper['ws'].frames[-1])
    try:
        answer = await request('scroll', lines=-1)
        assert answer['type']=='view' and answer['available'] and answer['held'] and answer['offset']==1
        assert b'ROW-1098' in work['ws'].frames[-1] and b'ROW-1099' not in work['ws'].frames[-1]
        answer = await request('follow', input=base64.b64encode(b'once').decode())
        assert not answer['held'] and work['input']==b'once'
        page = await request('viewport')
        assert page['type']=='page' and page['rowCount']==1100
        assert (await request('ack', epoch=page['epoch']))['type']=='ack'
        assert (await request('keep', epoch=page['epoch']))['type']=='ack'
        await request('scroll', lines=-2)
        assert (await request('close'))['type']=='ack' and work['nativeOwner'] is None and work['nativeOffset']==0
    finally:
        del fixture.sessions['viewport-smoke']


def main():
    screenshots()
    with tempfile.TemporaryDirectory(prefix='art-term-fixture-smoke-') as directory:
        fixture = Fixture(Path(directory))
        try:
            asyncio.run(pty(fixture))
            asyncio.run(viewport(fixture))
        finally:
            fixture.close()
            assert not fixture.thread.is_alive() and fixture.loop.is_closed()
    print('PASS fixture-only JSON/SSE PNG decode, TLS/WSS bootstrap, READY/configure/clear, resize, nativeViewport scroll/follow and legacy snapshot ops, exit4004, owned threads/sockets closed; Windows GUI NOT_RUN')


if __name__ == '__main__':
    main()
