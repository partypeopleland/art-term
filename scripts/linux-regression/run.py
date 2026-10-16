#!/usr/bin/env python3
"""Native official Runtime/helper regression; writes only repo target/.

python3 scripts/linux-regression/run.py --binary /absolute/official-openab-pty
Optional --live-mcp-config points to an existing owner-only desktop.env, never
copied or printed. No systemd, Tailnet, production Runtime or desktop mutations.
"""
import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import random
import re
import secrets
import shutil
import signal
import socket
import stat
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
import zlib

REPO = Path(__file__).resolve().parents[2]
ASSETS = REPO / 'scripts/openab-runtime'
sys.path.insert(0, str(ASSETS))
import official_artifact as artifact


def require(value, message):
    if not value:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def png_image():
    def chunk(kind, data):
        return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data))
    rng = random.Random(47)
    rows = b''.join(b'\0' + rng.randbytes(512 * 3) for _ in range(256))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!IIBBBBB', 512, 256, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def validate_png(raw):
    require(raw.startswith(b'\x89PNG\r\n\x1a\n'), 'PNG signature')
    pos, compressed, dimensions, ended = 8, bytearray(), None, False
    while pos < len(raw):
        require(pos + 12 <= len(raw), 'PNG truncated chunk')
        size = int.from_bytes(raw[pos:pos + 4], 'big')
        kind, data = raw[pos + 4:pos + 8], raw[pos + 8:pos + 8 + size]
        require(pos + size + 12 <= len(raw), 'PNG truncated data')
        crc = int.from_bytes(raw[pos + size + 8:pos + size + 12], 'big')
        require(zlib.crc32(kind + data) == crc, 'PNG CRC')
        if kind == b'IHDR':
            require(dimensions is None and size == 13, 'PNG IHDR')
            width, height, depth, color, compression, filtering, interlace = struct.unpack('!IIBBBBB', data)
            require(0 < width <= 4096 and 0 < height <= 4096 and (depth, color, compression, filtering, interlace) == (8, 2, 0, 0, 0), 'fixture PNG format')
            dimensions = (width, height)
        elif kind == b'IDAT':
            compressed.extend(data)
        elif kind == b'IEND':
            require(size == 0, 'PNG IEND')
            ended = True
        pos += size + 12
        if ended:
            break
    require(ended and pos == len(raw) and dimensions, 'PNG completion')
    expected = dimensions[1] * (1 + dimensions[0] * 3)
    inflater = zlib.decompressobj()
    pixels = inflater.decompress(compressed, expected + 1)
    require(len(pixels) == expected and inflater.eof and not inflater.unused_data, 'PNG decoded rows')
    require(all(pixels[row * (dimensions[0] * 3 + 1)] == 0 for row in range(dimensions[1])), 'PNG row filters')
    return dimensions


class McpHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        if self.path != '/mcp' or self.headers.get('Authorization') != 'Bearer ' + self.server.token:
            self.send_error(401)
            return
        length = int(self.headers.get('Content-Length', '0'))
        if not 0 < length <= 65536:
            self.send_error(400)
            return
        request = json.loads(self.rfile.read(length))
        method = request.get('method')
        if method == 'notifications/initialized':
            self.send_response(202)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        if method == 'initialize':
            result = {'protocolVersion': '2025-03-26', 'capabilities': {}, 'serverInfo': {'name': 'isolated-mcp', 'version': '1'}}
        elif method == 'tools/call' and request.get('params') == {'name': 'screenshot', 'arguments': {}}:
            self.server.calls += 1
            result = {'content': [{'type': 'image', 'mimeType': 'image/png', 'data': base64.b64encode(self.server.png).decode()}]}
        else:
            self.send_error(400)
            return
        raw = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


class WS:
    def __init__(self, port, name, token, since=0):
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=5)
        self.pending = b''
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        self.sock.sendall((f'GET /pty/{name}?since={since} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n'
                           'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n'
                           f'Sec-WebSocket-Key: {key}\r\nAuthorization: Bearer {token}\r\n\r\n').encode())
        while b'\r\n\r\n' not in self.pending:
            self.need(len(self.pending) + 1)
            require(len(self.pending) < 65536, 'WS handshake bound')
        header, self.pending = self.pending.split(b'\r\n\r\n', 1)
        require(header.startswith(b'HTTP/1.1 101 '), 'WS upgrade')
        notice = self.recv(5)
        require(notice[0] == 1, 'attach notice opcode')
        self.notice = json.loads(notice[1])
        require(self.notice.get('type') == 'attach-notice', 'attach notice')

    def need(self, count):
        while len(self.pending) < count:
            part = self.sock.recv(max(4096, min(65536, count - len(self.pending))))
            require(part, 'WS ended early')
            self.pending += part

    def send(self, opcode, raw):
        mask = secrets.token_bytes(4)
        size = len(raw)
        head = bytes([0x80 | opcode, 0x80 | size]) if size < 126 else bytes([0x80 | opcode, 0xfe]) + struct.pack('!H', size)
        self.sock.sendall(head + mask + bytes(v ^ mask[i % 4] for i, v in enumerate(raw)))

    def control(self, **value):
        self.send(1, json.dumps({'v': 1, **value}).encode())

    def recv(self, timeout):
        self.sock.settimeout(max(.01, timeout))
        while True:
            self.need(2)
            first, second = self.pending[:2]
            size, offset = second & 127, 2
            require(first & 0x80 and not second & 0x80, 'server frame flags')
            if size in (126, 127):
                width = 2 if size == 126 else 8
                self.need(2 + width)
                size = int.from_bytes(self.pending[2:2 + width], 'big')
                offset += width
            require(size <= 12 * 1024 * 1024, 'WS frame bound')
            self.need(offset + size)
            raw, self.pending = self.pending[offset:offset + size], self.pending[offset + size:]
            opcode = first & 15
            if opcode == 9:
                self.send(10, raw)
                continue
            return opcode, raw

    def collect(self, needle, timeout=25):
        output = bytearray()
        deadline = time.monotonic() + timeout
        while needle not in output:
            require(time.monotonic() < deadline, 'PTY deadline')
            opcode, raw = self.recv(deadline - time.monotonic())
            if opcode == 2:
                output.extend(raw)
                require(len(output) <= 12 * 1024 * 1024, 'PTY output bound')
            elif opcode == 1:
                require(json.loads(raw).get('type') != 'gap', 'PTY gap')
            else:
                require(opcode != 8, 'PTY premature close')
        return bytes(output)

    def close(self):
        self.sock.close()


class Harness:
    def __init__(self, root, binary):
        self.root, self.owned, self.sockets = root, set(), []
        self.home = root / 'home'
        self.home.mkdir(mode=0o700)
        self.tmp = root / 'tmp'
        self.tmp.mkdir(mode=0o700)
        self.helper = self.home / '.local/share/art-term/openab-pty'
        self.helper.mkdir(parents=True, mode=0o700)
        for name in ('preview.py', 'mcp_preview.py'):
            shutil.copyfile(ASSETS / name, self.helper / name)
            (self.helper / name).chmod(0o700)
        self.binary = root / 'openab-pty'
        raw = binary.read_bytes() if binary else artifact.fetch()
        require(len(raw) == artifact.BINARY_BYTES and digest(raw) == artifact.BINARY_SHA256, 'pinned official binary mismatch')
        self.binary.write_bytes(raw)
        self.binary.chmod(0o700)
        self.env = {'HOME': str(self.home), 'TMPDIR': str(self.tmp), 'PATH': '/usr/local/bin:/usr/bin:/bin',
                    'USER': str(os.getuid()), 'SHELL': '/bin/bash', 'TERM': 'xterm-256color', 'LANG': 'C.UTF-8'}
        generated = subprocess.run([str(self.binary), '--generate-admin-credential'], env=self.env, capture_output=True, timeout=10)
        require(generated.returncode == 0 and len(generated.stdout) < 4096, 'credential generation')
        match = re.search(rb'^credential: ([a-f0-9]{64})$', generated.stdout, re.M)
        require(match, 'credential format')
        self.credential = match[1].decode()
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), McpHandler)
        self.server.daemon_threads = True
        self.server.token, self.server.png, self.server.calls = secrets.token_hex(32), png_image(), 0
        require(len(self.server.png) > 256 * 1024, 'large PNG fixture')
        validate_png(self.server.png)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        config = self.helper / 'desktop.env'
        config.write_text(f'ART_TERM_DESKTOP_MCP_URL=http://127.0.0.1:{self.server.server_port}/mcp\nHANDS_MCP_TOKEN={self.server.token}\n')
        config.chmod(0o600)
        shell = root / 'shell'
        shell.write_text('#!/bin/bash\nexec /bin/bash --noprofile --norc -i\n')
        shell.chmod(0o700)
        self.port = free_port()
        projection = root / 'config.toml'
        projection.write_text(f'[pty]\nenabled=true\nlisten="127.0.0.1:{self.port}"\ntls_terminated_upstream=true\ncommand="{shell}"\nmax_sessions=8\nscrollback_kib=1024\nscrollback_replay=false\nadmin_credential_hash="sha256:{digest(self.credential.encode())}"\n')
        projection.chmod(0o600)
        checked = subprocess.run([str(self.binary), '--validate-projection', str(projection)], env=self.env, capture_output=True, timeout=10)
        require(checked.returncode == 0, 'runtime projection')
        self.process = subprocess.Popen([str(self.binary), '--config', str(projection)], cwd=root, env=self.env,
                                        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        self.birth = self.starttime()
        deadline = time.monotonic() + 10
        while True:
            require(self.process.poll() is None, 'runtime exited during startup (possible port race)')
            try:
                if self.api('GET', '/admin/sessions')[0] == 200:
                    self.ready = True
                    break
            except (OSError, urllib.error.URLError):
                pass
            require(time.monotonic() < deadline, 'runtime readiness deadline')
            time.sleep(.05)

    def starttime(self):
        return Path(f'/proc/{self.process.pid}/stat').read_text().rsplit(')', 1)[1].split()[19]

    def api(self, method, path, body=None, credential=None):
        request = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}',
                  json.dumps(body).encode() if body is not None else None,
                  {'Authorization': 'Bearer ' + (credential if credential is not None else self.credential), 'Content-Type': 'application/json'}, method=method)
        try:
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=15 if method == 'DELETE' else 5) as response:
                return response.status, json.loads(response.read(1024 * 1024))
        except urllib.error.HTTPError as error:
            return error.code, {}

    def create(self, prefix):
        name = prefix + secrets.token_hex(6)
        status, grant = self.api('POST', '/admin/sessions', {'name': name})
        require(status == 200, 'create owned session')
        self.owned.add(name)
        return name, grant

    def attach(self, name, grant, since=0):
        ws = WS(self.port, name, grant['token'], since)
        self.sockets.append(ws)
        return ws

    def delete(self, name):
        require(name in self.owned, 'refuse unowned DELETE')
        # Runtime removes exited children itself; a recorded owned helper may
        # already be absent by the time fixture teardown reaches it.
        require(self.api('DELETE', '/admin/sessions/' + name)[0] in (200, 404), 'owned DELETE')
        self.owned.discard(name)

    def preview(self, work):
        name, grant = self.create('mcshot-')
        ws = self.attach(name, grant)
        nonce = secrets.token_hex(16)
        ws.send(2, ('python3 "$HOME/.local/share/art-term/openab-pty/preview.py" screenshot ' + nonce + '\n').encode())
        raw = ws.collect(('ARTTERM_PREVIEW_END:' + nonce + ':0').encode())
        lines = raw.replace(b'\r\n', b'\n').splitlines()
        begin = lines.index(('ARTTERM_PREVIEW_BEGIN:' + nonce).encode())
        require(lines[begin + 2] == ('ARTTERM_PREVIEW_END:' + nonce + ':0').encode(), 'preview framing')
        value = json.loads(lines[begin + 1])
        image = base64.b64decode(value['image_base64'], validate=True)
        require(value['bytes'] == len(image) and value['mime'] == 'image/png' and image == self.server.png, 'preview image integrity')
        dimensions = validate_png(image)
        require(self.server.calls == 1, 'real helper MCP screenshot call')
        require(self.credential.encode() not in raw and self.server.token.encode() not in raw, 'secret in preview output')
        ws.control(type='detach')
        ws.close()
        self.delete(name)
        work.send(2, b"printf '\\x49\\x53\\x4f\\x4c\\x41\\x54\\x45\\x44\\n'\n")
        observed = work.collect(b'ISOLATED\r\n')
        require(not any(marker in observed for marker in (nonce.encode(), b'preview.py', b'image_base64', b'ARTTERM_PREVIEW_')), 'work/screenshot isolation')
        return {'passed': True, 'backend': 'fake MCP / real helper / real official PTY', 'imageBytes': len(image), 'imageSha256': digest(image), 'dimensions': dimensions}

    def runtime(self):
        require(self.api('GET', '/admin/sessions', credential='invalid')[0] == 401, 'wrong credential must fail')
        require(self.api('GET', '/admin/sessions')[1]['sessions'] == [], 'isolated baseline')
        name, grant = self.create('work-')
        ws = self.attach(name, grant)
        ws.control(type='resize', rows=35, cols=100)
        ws.send(2, b"stty -echo; stty size; printf '\\344\\270\\255\\346\\226\\207-RUNTIME\\n'\n")
        raw = ws.collect('中文-RUNTIME\r\n'.encode())
        require(b'35 100' in raw, 'real PTY resize')
        require(self.credential.encode() not in raw, 'secret in work output')
        preview = self.preview(ws)
        # Drain prompt before recording the actual stream cursor.
        deadline = time.monotonic() + .25
        while time.monotonic() < deadline:
            try:
                ws.recv(deadline - time.monotonic())
            except socket.timeout:
                break
        listing = self.api('GET', '/admin/sessions')[1]['sessions']
        current = next(s for s in listing if s['name'] == name)
        cursor = current['bytes_written']
        ws.control(type='detach')
        ws.close()
        time.sleep(.1)
        resumed = self.attach(name, grant, cursor)
        resumed.send(2, b"printf '\\x52\\x45\\x53\\x55\\x4d\\x45\\x44\\n'\n")
        fresh = resumed.collect(b'RESUMED\r\n')
        require('中文-RUNTIME'.encode() not in fresh and b'ISOLATED' not in fresh, 'resume old output replay')
        listing = self.api('GET', '/admin/sessions')[1]['sessions']
        require(next(s for s in listing if s['name'] == name)['generation'] == grant['generation'], 'resume generation changed')
        resumed.control(type='detach')
        resumed.close()
        self.delete(name)
        require(self.api('GET', '/admin/sessions')[1]['sessions'] == [], 'final sessions baseline')
        return {'passed': True, 'unicode': True, 'resize': [35, 100], 'sameTokenResume': True, 'cursor': cursor, 'renewCalls': 0, 'noOldReplay': True}, preview

    def live(self, path):
        info = path.lstat()
        require(path.is_absolute() and stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and not stat.S_IMODE(info.st_mode) & 0o077, 'live config must be owner-only regular file')
        env = {**self.env, 'ART_TERM_PREVIEW_CONFIG': str(path)}
        nonce = secrets.token_hex(16)
        result = subprocess.run([sys.executable, str(self.helper / 'preview.py'), 'screenshot', nonce], env=env, capture_output=True, timeout=25)
        require(result.returncode == 0 and len(result.stdout) <= 12 * 1024 * 1024 and not result.stderr, 'live MCP screenshot failed')
        # preview.py writes exactly one leading newline, then three lines.
        require(result.stdout.startswith(b'\n'), 'live MCP leading newline')
        lines = result.stdout[1:].splitlines()
        require(len(lines) == 3 and lines[0] == ('ARTTERM_PREVIEW_BEGIN:' + nonce).encode()
                and lines[2] == ('ARTTERM_PREVIEW_END:' + nonce + ':0').encode(), 'live MCP framing')
        value = json.loads(lines[1])
        image = base64.b64decode(value['image_base64'], validate=True)
        require(len(image) == value['bytes'] and 0 < len(image) <= 8 * 1024 * 1024, 'live image bytes')
        require(value['mime'] in ('image/png', 'image/jpeg'), 'live image MIME')
        return {'passed': True, 'backend': 'existing MCP screenshot only', 'validation': 'capture / helper signature check; not full image decode',
                'imageBytes': len(image), 'imageSha256': digest(image), 'mime': value['mime']}

    def close(self):
        failures = []
        for ws in getattr(self, 'sockets', []):
            ws.close()
        for name in list(getattr(self, 'owned', [])):
            try:
                self.delete(name)
            except Exception:
                failures.append('owned session delete')
        if getattr(self, 'ready', False):
            try:
                require(self.api('GET', '/admin/sessions')[1]['sessions'] == [], 'cleanup sessions')
            except Exception:
                failures.append('session baseline')
        # Even failed API cleanup must still terminate the child we own.
        if hasattr(self, 'process') and self.process.poll() is None:
            require(self.starttime() == self.birth and os.getpgid(self.process.pid) == self.process.pid, 'runtime identity changed')
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                require(self.starttime() == self.birth, 'runtime identity changed before kill')
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        if hasattr(self, 'server'):
            if hasattr(self, 'thread') and self.thread.is_alive():
                self.server.shutdown()
                self.thread.join(timeout=3)
            self.server.server_close()
        if hasattr(self, 'helper'):
            (self.helper / 'desktop.env').unlink(missing_ok=True)
        if hasattr(self, 'root'):
            (self.root / 'config.toml').unlink(missing_ok=True)
        if hasattr(self, 'process'):
            with socket.socket() as sock:
                require(sock.connect_ex(('127.0.0.1', self.port)) != 0, 'owned runtime port still open')
        require(not failures, 'cleanup failed: ' + ', '.join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--live-mcp-config', type=Path)
    parser.add_argument('--host-fixtures', action='store_true', help='also run launcher/desktop/upgrade/init-host suites')
    args = parser.parse_args()
    require(sys.platform == 'linux' and os.getuid() != 0, 'ordinary Linux user required')
    require(args.binary is None or args.binary.is_absolute() and args.binary.is_file() and not args.binary.is_symlink(), 'absolute regular binary required')
    os.umask(0o077)
    root = REPO / 'target/linux-regression' / ('run-' + uuid.uuid4().hex)
    root.mkdir(parents=True, mode=0o700)
    source_paths = [Path(__file__).resolve(), REPO / 'scripts/test-init-host.py', REPO / 'scripts/init-host.sh']
    source_paths += sorted(ASSETS.glob('*.py'))
    if args.host_fixtures:
        source_paths += [REPO / 'scripts' / name for name in ('test-openab-official.py', 'test-openab-desktop.py', 'test-openab-upgrade.py')]
    result = {'passed': False, 'binarySha256': artifact.BINARY_SHA256,
              'sourceHashes': {str(path.relative_to(REPO)): digest(path.read_bytes()) for path in source_paths},
              'cases': {}, 'cleanup': False}
    harness = Harness.__new__(Harness)
    try:
        harness.__init__(root, args.binary)
        runtime, preview = harness.runtime()
        result['cases'].update({'L-RUNTIME': runtime, 'L-PREVIEW': preview})
        if args.live_mcp_config:
            result['cases']['L-LIVE-MCP'] = harness.live(args.live_mcp_config)
        if args.host_fixtures:
            checks = []
            result['cases']['L-HOST'] = {'passed': False, 'checks': checks, 'serviceBackend': 'fixture adapters; not production systemd'}
            for script in ('test-openab-official.py', 'test-openab-desktop.py', 'test-openab-upgrade.py', 'test-init-host.py'):
                command = [sys.executable, str(REPO / 'scripts' / script)]
                if script == 'test-init-host.py':
                    command += ['--binary', str(harness.binary), '--port', str(free_port())]
                completed = subprocess.run(command, env={**os.environ, 'TMPDIR': str(harness.tmp)}, capture_output=True, timeout=240)
                summary = re.search(rb'Ran (\d+) tests? in ', completed.stderr)
                checks.append({'script': script, 'passed': completed.returncode == 0, 'exitCode': completed.returncode,
                               'testsRun': int(summary[1]) if summary else None})
                require(completed.returncode == 0, 'Host fixture failed: ' + script)
            result['cases']['L-HOST']['passed'] = True
        result['passed'] = True
    except Exception as error:
        result['failure'] = type(error).__name__ + ': ' + str(error) if isinstance(error, RuntimeError) else type(error).__name__
    finally:
        try:
            harness.close()
            result['cleanup'] = True
        except Exception as error:
            result['cleanupFailure'] = type(error).__name__
            result['passed'] = False
        (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'passed': result['passed'], 'cleanup': result['cleanup'], 'result': str(root / 'result.json')}))
    return 0 if result['passed'] and result['cleanup'] else 1


if __name__ == '__main__':
    sys.exit(main())
