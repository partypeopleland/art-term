#!/usr/bin/env python3
"""App-selected MCP tools. No persisted upstream URL, token or CLI config."""
import argparse
import base64
import hashlib
import http.client
import ipaddress
import json
import os
from pathlib import Path
import select
import socket
import socketserver
import stat
import struct
import sys
import termios
import threading
import time
import urllib.parse
import uuid

LIMIT = 16 * 1024 * 1024
ERROR = 'instance-mcp request failed'


def require(ok):
    if not ok:
        raise ValueError(ERROR)


class Mcp:
    def __init__(self, config, timeout=60):
        raw_url = config['url']
        require(isinstance(raw_url, str) and raw_url.strip() == raw_url and '\\' not in raw_url and all(ord(c) >= 32 and ord(c) != 127 for c in raw_url) and '@' not in raw_url.split('://', 1)[-1].split('/', 1)[0])
        self.url = urllib.parse.urlsplit(raw_url)
        u = self.url
        require(u.scheme in ('http', 'https') and u.hostname and not u.username and not u.password
                and not u.query and not u.fragment and u.path == '/mcp' and len(config['url']) <= 2048)
        self.token = config['token']
        require(isinstance(self.token, str) and 0 < len(self.token) <= 1024 and self.token.isascii()
                and all(32 < ord(c) < 127 for c in self.token))
        self.deadline = time.monotonic() + timeout
        self.session, self.version, self.counter = None, '2025-03-26', 0
        try:
            init = self.call('initialize', {'protocolVersion': self.version, 'capabilities': {},
                             'clientInfo': {'name': 'art-term-tools', 'version': '1'}})
            negotiated = init.get('protocolVersion')
            require(negotiated in ('2024-11-05', '2025-03-26', '2025-06-18', '2025-11-25'))
            self.version = negotiated
            self.request({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        except Exception:
            self.close()
            raise

    def connection(self):
        u = self.url
        remaining = self.deadline - time.monotonic()
        require(remaining > 0)
        if u.scheme == 'https':
            return http.client.HTTPSConnection(u.hostname, u.port or 443, timeout=remaining)
        import queue
        resolved = queue.Queue(1)
        def lookup():
            try: resolved.put(socket.getaddrinfo(u.hostname, u.port or 80, type=socket.SOCK_STREAM))
            except Exception: resolved.put(None)
        threading.Thread(target=lookup, daemon=True).start()
        addresses = resolved.get(timeout=max(.01, self.deadline-time.monotonic()))
        require(addresses)
        for a in addresses:
            ip = ipaddress.ip_address(a[4][0])
            require(ip.is_loopback or ip in ipaddress.ip_network('100.64.0.0/10') or ip in ipaddress.ip_network('fd7a:115c:a1e0::/48'))
        # Pin the resolved address; never resolve the hostname again in connect.
        return http.client.HTTPConnection(addresses[0][4][0], u.port or 80, timeout=remaining)

    def request(self, body=None, method='POST'):
        conn = self.connection()
        try:
            headers = {'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json',
                       'Accept': 'application/json, text/event-stream', 'Host': self.url.netloc,
                       'MCP-Protocol-Version': self.version}
            if self.session:
                headers['Mcp-Session-Id'] = self.session
            conn.request(method, self.url.path, json.dumps(body).encode() if body else None, headers)
            r = conn.getresponse()
            require(200 <= r.status < 300)
            session = r.getheader('Mcp-Session-Id')
            if session:
                require(len(session) <= 1024 and all(32 < ord(c) < 127 for c in session))
                self.session = session
            if method == 'DELETE' or 'id' not in body:
                return None
            content_type = r.getheader('Content-Type', '').split(';')[0].strip().lower()
            require(content_type in ('application/json', 'text/event-stream'))
            sse = content_type == 'text/event-stream'
            raw, event = b'', []
            while True:
                require(time.monotonic() < self.deadline)
                if conn.sock:
                    conn.sock.settimeout(max(.01, self.deadline-time.monotonic()))
                chunk = r.readline(LIMIT-len(raw)+1) if sse else r.read1(min(65536, LIMIT-len(raw)+1))
                raw += chunk
                require(len(raw) <= LIMIT)
                if sse:
                    if chunk.strip() == b'':
                        data = b'\n'.join(event); event = []
                        if data:
                            reply = json.loads(data)
                            if reply.get('id') == body['id']:
                                break
                    elif chunk.startswith(b'data:'):
                        event.append(chunk[5:].lstrip().rstrip(b'\r\n'))
                elif not chunk:
                    reply = json.loads(raw)
                    break
                require(chunk)
            require(reply.get('jsonrpc') == '2.0' and reply.get('id') == body['id'] and 'error' not in reply and isinstance(reply.get('result'), dict))
            # Upstream may be untrusted; never propagate credential readback.
            def contains_token(value):
                if isinstance(value, str): return self.token in value
                if isinstance(value, list): return any(contains_token(item) for item in value)
                if isinstance(value, dict): return any(contains_token(key) or contains_token(item) for key,item in value.items())
                return False
            require(not contains_token(reply))
            return reply['result']
        finally:
            conn.close()

    def call(self, method, params):
        self.counter += 1
        return self.request({'jsonrpc': '2.0', 'id': self.counter, 'method': method, 'params': params})

    def close(self):
        if self.session:
            try:
                self.deadline = time.monotonic()+2
                self.request(method='DELETE')
            except Exception:
                pass


def private_directory(path):
    try: path.mkdir(mode=0o700)
    except FileExistsError: pass
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        require(info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) in (0o700, 0o2700))
        # fsGroup/setgid parents may propagate SGID even with mkdir(mode=700).
        # Strip only that safe inherited bit; reject any group/other access.
        if stat.S_IMODE(info.st_mode) == 0o2700: os.fchmod(fd, 0o700)
        require(stat.S_IMODE(os.fstat(fd).st_mode) == 0o700)
    finally:
        os.close(fd)


def directory(binding):
    require(isinstance(binding, str) and 0 < len(binding) <= 32 and all(c in 'abcdefghijklmnopqrstuvwxyz0123456789-' for c in binding))
    runtime = os.environ.get('XDG_RUNTIME_DIR')
    if runtime:
        parent = Path(runtime)
        require(parent.is_absolute())
        info = parent.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == 0o700)
        root = parent / 'art-term-tools'
    else:
        root = Path('/tmp') / ('art-term-' + str(os.getuid()))
    path = root / hashlib.sha256(binding.encode()).hexdigest()[:24]
    for p in (root, path):
        private_directory(p)
    return path


def receive(sock, maximum=LIMIT):
    def exact(count):
        out = bytearray()
        while len(out) < count:
            part = sock.recv(count-len(out)); require(part); out.extend(part)
        return bytes(out)
    size = struct.unpack('!I', exact(4))[0]; require(0 < size <= maximum)
    return json.loads(exact(size))


def send(sock, body):
    raw = json.dumps(body, separators=(',', ':')).encode(); require(len(raw) <= LIMIT)
    sock.sendall(struct.pack('!I', len(raw))+raw)


def legacy_current(binding, path):
    reply = broker_request(binding, {'op': 'rpc', 'request': {'method': 'tools/call', 'params': {'name': 'current_target', 'arguments': {}}}}, path)
    if reply.get('isError'):
        return None
    target = json.loads(reply['content'][0]['text'])['target']
    require(isinstance(target, dict) and isinstance(target.get('id'), str)
            and type(target.get('revision')) is int and type(target.get('generation')) is int)
    return target


def configure(binding, body):
    statuses = []
    confirmed = []
    paths = sorted(directory(binding).glob('*.sock'))
    for path in paths:
        info = path.lstat()
        require(stat.S_ISSOCK(info.st_mode) and info.st_uid == os.getuid())
    for path in paths:
        s = socket.socket(socket.AF_UNIX); s.settimeout(20)
        try:
            s.connect(str(path)); send(s, body)
            status = receive(s, 65536)
            # A Shell created before this helper update still has the older
            # broker in memory. Its existing MCP current_target and fenced
            # clear operations are enough to recover without restarting it.
            if not status.get('ok') and body.get('op') in ('current', 'clear_stale'):
                target = legacy_current(binding, path)
                if body['op'] == 'clear_stale' and target in body.get('expected_targets', []):
                    status = broker_request(binding, {'op': 'clear', 'expected_target': target}, path)
                else:
                    status = {'ok': True, 'target': target}
            statuses.append(status)
            if status.get('ok') and status.get('target'):
                confirmed.append((path, status['target']))
        except (ConnectionRefusedError, FileNotFoundError):
            path.unlink(missing_ok=True)
        except Exception:
            statuses.append({'ok': False})
        finally:
            s.close()
    if not all(v.get('ok') for v in statuses):
        # Roll back only our confirmed generation, never a newer selection.
        for path, expected in (confirmed if body.get('op') == 'configure' else []):
            try:
                with socket.socket(socket.AF_UNIX) as client:
                    client.settimeout(2); client.connect(str(path))
                    send(client, {'op': 'clear', 'expected_target': expected}); receive(client, 65536)
            except Exception:
                pass
        raise ValueError(ERROR)
    return {'ok': True, 'adapters': len(statuses), 'targets': [s.get('target') for s in statuses],
            'urls': [s.get('url') for s in statuses]}


def text(value):
    return {'content': [{'type': 'text', 'text': json.dumps(value, separators=(',', ':'))}]}


TOOLS = [
        {'name': 'current_target', 'description': 'Read the App-selected instance-mcp target identity. Use its identity for all calls.', 'inputSchema': {'type': 'object', 'properties': {}, 'additionalProperties': False}},
        {'name': 'list_target_tools', 'description': 'Discover the selected instance-mcp tools and their original names/input schemas. Returns target identity and cursor.', 'inputSchema': {'type': 'object', 'properties': {'cursor': {'type': 'string'}}, 'additionalProperties': False}},
        {'name': 'call_target_tool', 'description': 'Execute any discovered instance-mcp desktop/browser tool. expected_target must match current_target; stale selection is rejected. Results retain source target.', 'inputSchema': {'type': 'object', 'properties': {'name': {'type': 'string'}, 'arguments': {'type': 'object'}, 'expected_target': {'type': 'object'}}, 'required': ['name', 'arguments', 'expected_target'], 'additionalProperties': False}},
    ]

DENIED = {'isError': True, 'content': [{'type': 'text', 'text': 'No ready target, stale expected_target, or instance-mcp request failed. Read current_target and select a desktop in the App.'}]}

class Broker:
    def __init__(self):
        self.lock = threading.Lock()
        self.selection = {'config': None, 'generation': 0, 'request_generation': 0}

    @staticmethod
    def target(config, generation):
        return {'id': config['id'], 'revision': config['revision'], 'generation': generation} if config else None

    def configure(self, body):
        if body.get('op') == 'current':
            with self.lock:
                config = self.selection['config']
                return {'ok': True, 'target': self.target(config, self.selection['generation']),
                        'url': config['url'] if config else None}
        if body.get('op') == 'clear_stale':
            with self.lock:
                current = self.target(self.selection['config'], self.selection['generation'])
                expected = body.get('expected_targets')
                require(isinstance(expected, list) and len(expected) <= 64)
                if current and current in expected:
                    self.selection['config'] = None
                    self.selection['generation'] += 1
                    self.selection['request_generation'] += 1
                    current = None
                return {'ok': True, 'target': current}
        if body.get('op') == 'clear' and 'expected_target' in body:
            with self.lock:
                current = self.target(self.selection['config'], self.selection['generation'])
                if current == body['expected_target']:
                    self.selection['config'] = None; self.selection['generation'] += 1
                    current = None
            return {'ok': True, 'target': current}
        with self.lock:
            self.selection['request_generation'] += 1
            ticket = self.selection['request_generation']
        if body['op'] == 'configure':
            require(isinstance(body['id'], str) and type(body['revision']) is int and body['revision'] > 0)
            m = Mcp(body, 15)
            try:
                m.call('tools/list', {})
            finally:
                m.close()
            config = {k: body[k] for k in ('id', 'revision', 'name', 'url', 'token')}
        elif body['op'] == 'clear':
            config = None
        else:
            raise ValueError(ERROR)
        with self.lock:
            require(ticket == self.selection['request_generation'])
            self.selection['config'] = config; self.selection['generation'] += 1
            result = self.target(config, self.selection['generation'])
        return {'ok': True, 'target': result}

    def rpc(self, request):
        try:
            method = request['method']; params = request.get('params', {})
            if method == 'initialize':
                result = {'protocolVersion': '2025-03-26', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'art-term-tools', 'version': '1'}, 'instructions': 'Read current_target and list_target_tools before operations. Always pass the returned expected_target. Selection may change in the App.'}
            elif method == 'ping':
                result = {}
            elif method == 'tools/list':
                result = {'tools': TOOLS}
            elif method == 'tools/call':
                with self.lock:
                    config = dict(self.selection['config']) if self.selection['config'] else None
                    t = self.target(config, self.selection['generation'])
                require(config)
                name, args = params['name'], params.get('arguments', {})
                if name == 'current_target':
                    result = text({'target': t, 'name': config['name']})
                else:
                    require(name in ('list_target_tools', 'call_target_tool'))
                    if name == 'call_target_tool':
                        require(args.get('expected_target') == t)
                    m = Mcp(config, 15 if name == 'list_target_tools' else 60)
                    try:
                        if name == 'list_target_tools':
                            r = m.call('tools/list', {k: args[k] for k in ('cursor',) if k in args})
                            result = text({'target': t, **r})
                        else:
                            result = m.call('tools/call', {'name': args['name'], 'arguments': args['arguments']})
                            require(isinstance(result.get('content'), list))
                            # Preserve upstream content/isError/structuredContent unchanged.
                        result.setdefault('_meta', {})['artTermTarget'] = t
                    finally:
                        m.close()
            else:
                raise ValueError(ERROR)
            return result
        except Exception:
            return dict(DENIED)

    def close(self):
        with self.lock:
            self.selection['config'] = None
            self.selection['request_generation'] += 1


def checked_socket(path):
    info = path.lstat()
    require(stat.S_ISSOCK(info.st_mode) and info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == 0o600)


def process_identity(pid):
    base = Path('/proc') / str(pid)
    info = base.stat()
    fields = (base/'stat').read_text().rsplit(')', 1)[1].split()
    require(info.st_uid == os.getuid())
    return int(fields[1]), fields[19]


def ancestor_binding(pid):
    # Stream only the requested nonsecret field; discard other values without
    # decoding or retaining them. Bound total environment bytes and field size.
    key = b'ART_TERM_SCREEN_BINDING='
    value = bytearray(); prefix = bytearray(); matches = []
    matched = False; discarded = False; count = 0
    with (Path('/proc')/str(pid)/'environ').open('rb', buffering=0) as stream:
        while True:
            chunk = stream.read(4096)
            if not chunk: break
            count += len(chunk); require(count <= 262144)
            for byte in chunk:
                if byte == 0:
                    if matched:
                        require(len(value) <= 32)
                        matches.append(value.decode('ascii'))
                    value.clear(); prefix.clear(); matched = discarded = False
                elif matched:
                    value.append(byte); require(len(value) <= 32)
                elif not discarded:
                    prefix.append(byte)
                    if not key.startswith(prefix): discarded = True; prefix.clear()
                    elif prefix == key: matched = True; prefix.clear()
    require(len(matches) <= 1)
    return matches[0] if matches else None


def shell_candidates(binding, deadline):
    require(isinstance(binding,str) and 0 < len(binding) <= 32 and all(c in 'abcdefghijklmnopqrstuvwxyz0123456789-' for c in binding))
    suffix = hashlib.sha256(binding.encode()).hexdigest()[:24]
    roots = [Path('/tmp')/('art-term-'+str(os.getuid())), Path('/run/user')/str(os.getuid())/'art-term-tools']
    if os.environ.get('XDG_RUNTIME_DIR'):
        require(Path(os.environ['XDG_RUNTIME_DIR']).is_absolute())
        roots.append(Path(os.environ['XDG_RUNTIME_DIR'])/'art-term-tools')
    found = []
    for root in dict.fromkeys(roots):
        path = root/suffix/'shell.sock'
        if not path.exists(): continue
        for parent in (root.parent, root, path.parent):
            info = parent.lstat()
            require(stat.S_ISDIR(info.st_mode))
            if parent != Path('/tmp'):
                require(info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == 0o700)
        checked_socket(path)
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(max(.01, deadline-time.monotonic()));client.connect(str(path))
            peer,uid,_ = struct.unpack('3i',client.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))
            require(uid == os.getuid())
            found.append((path, (peer, process_identity(peer))))
    return found


def current_shell():
    deadline = time.monotonic()+1
    ancestors = {}; bindings = set(); candidates = []; found = None
    direct = os.environ.get('ART_TERM_SCREEN_BINDING')
    if direct: bindings.add(direct)
    pid = os.getpid()
    for _ in range(32):
        require(time.monotonic() < deadline and pid not in ancestors)
        before = process_identity(pid)
        ancestors[pid] = before
        matching = [item for item in candidates if item[1] == (pid, before)]
        if matching:
            # This authenticated broker owns the relevant Shell ancestry.
            # Do not inspect its service manager's unrelated /proc environment.
            require(len(candidates) == 1 and len(matching) == 1)
            found = matching[0]
            break
        value = ancestor_binding(pid)
        require(process_identity(pid) == before)
        if value: bindings.add(value)
        require(len(bindings) <= 1)
        if bindings and not candidates:
            candidates = shell_candidates(next(iter(bindings)), deadline)
        if before[0] <= 1: break
        pid = before[0]
        if (Path('/proc')/str(pid)).stat().st_uid != os.getuid(): break
    require(found and len(bindings) == 1 and time.monotonic() < deadline)
    require(all(process_identity(pid) == expected for pid,expected in ancestors.items()))
    return bindings.pop(), found[0], found[1]


def broker_request(binding, body, path=None, expected_peer=None):
    path = path or directory(binding) / 'shell.sock'
    checked_socket(path)
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(75); client.connect(str(path))
        peer,uid,_ = struct.unpack('3i', client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        require(uid == os.getuid())
        if expected_peer:
            require(peer == expected_peer[0] and process_identity(peer) == expected_peer[1])
        send(client, body)
        return receive(client)


def serve(binding, broker, shell=False):
    slots = threading.BoundedSemaphore(8)
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            if not slots.acquire(blocking=False):
                return
            try:
                self.request.settimeout(75)
                require(struct.unpack('3i', self.request.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[1] == os.getuid())
                body = receive(self.request, 65536)
                if body.get('op') == 'rpc':
                    require(shell)
                    result = broker.rpc(body['request'])
                else:
                    require(len(json.dumps(body).encode()) <= 16384)
                    result = broker.configure(body)
                send(self.request, result)
            except Exception:
                try: send(self.request, {'ok': False, 'error': ERROR})
                except OSError: pass
            finally:
                slots.release()
    class Server(socketserver.ThreadingUnixStreamServer):
        daemon_threads = True
        request_queue_size = 8
    path = directory(binding) / ('shell.sock' if shell else str(os.getpid())+'-'+uuid.uuid4().hex[:8]+'.sock')
    # Never replace an existing live or stale supervisor endpoint implicitly.
    server = Server(str(path), Handler)
    os.chmod(path, 0o600)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, path


def adapter(binding, required_remote=False, remote_path=None, expected_peer=None):
    # Scoped Shell clients require their broker; never create an independent grant.
    scoped = os.environ.get('ART_TERM_SCREEN_BINDING') == binding
    remote = required_remote or scoped or (directory(binding) / 'shell.sock').exists()
    broker = None if remote else Broker()
    server, path = (None, None) if remote else serve(binding, broker)
    try:
        while True:
            line = sys.stdin.buffer.readline(65537)
            if not line: break
            require(len(line) <= 65536)
            request = json.loads(line)
            if 'id' not in request: continue
            try:
                result = broker_request(binding, {'op':'rpc','request':request},remote_path,expected_peer) if remote else broker.rpc(request)
            except Exception:
                result = dict(DENIED)
            print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result},separators=(',',':')),flush=True)
    finally:
        if broker:
            broker.close(); server.shutdown(); server.server_close(); path.unlink(missing_ok=True)


def shell_command(binding, command):
    require(command in ('current', 'list', 'call', 'mcp-config'))
    if command == 'mcp-config':
        current_shell()
        print(json.dumps({'command':'python3','args':[str(Path(__file__).resolve()),'stdio','--current-shell']},separators=(',',':')))
        return 0
    args = {}
    if command == 'call':
        raw = sys.stdin.buffer.read(65537); require(len(raw) <= 65536)
        args = json.loads(raw)
    request = {'method':'tools/call', 'params':{'name':{'current':'current_target','list':'list_target_tools','call':'call_target_tool'}[command], 'arguments':args}}
    result = broker_request(binding, {'op':'rpc','request':request})
    print(json.dumps(result,separators=(',',':')))
    return 1 if result.get('isError') else 0


def control(binding, nonce):
    require(len(nonce) == 32 and all(c in '0123456789abcdef' for c in nonce))
    fd = sys.stdin.fileno()
    require(os.isatty(fd))
    # exec launcher means no interactive shell can consume residual input.
    attrs = termios.tcgetattr(fd)
    attrs[3] &= ~(termios.ECHO | termios.ECHONL | termios.ICANON | termios.ISIG)
    attrs[0] &= ~(termios.IXON | termios.ICRNL)
    attrs[6][termios.VMIN] = 1; attrs[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    print('\nARTTERM_TOOLS_READY:'+nonce, flush=True)
    deadline = time.monotonic()+10

    def exact(n):
        out = bytearray()
        while len(out) < n:
            require(select.select([fd], [], [], max(0, deadline-time.monotonic()))[0])
            part = os.read(fd, n-len(out)); require(part); out.extend(part)
        return bytes(out)

    size = struct.unpack('!I', exact(4))[0]; require(0 < size <= 16384)
    body = json.loads(base64.b64decode(exact(size), validate=True))
    result = configure(binding, body)
    print('ARTTERM_TOOLS_RESULT:'+nonce+':'+json.dumps(result, separators=(',', ':')), flush=True)


def cli_args(binding, command):
    require(command and Path(command[0]).name == 'codex')
    adapter_args = [str(Path(__file__).resolve()), 'stdio', '--binding', binding]
    return [command[0], '-c', 'mcp_servers.art_term.command="python3"', '-c', 'mcp_servers.art_term.args='+json.dumps(adapter_args), '-c', 'mcp_servers.art_term.tool_timeout_sec=75', *command[1:]]


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['stdio', 'control', 'launch', 'shell', 'shell-command'])
    parser.add_argument('--binding')
    parser.add_argument('--current-shell', action='store_true')
    parser.add_argument('--nonce')
    parser.add_argument('command', nargs='*')
    args, trailing = parser.parse_known_args()
    # Remove only retired fixed-desktop grants. Re-exec also replaces Linux's
    # original environment memory visible through /proc, before any broker exists.
    clean_env = {key:value for key,value in os.environ.items() if not key.startswith(('MY_CLI_DESKTOP_', 'ART_TERM_DESKTOP_')) and key != 'HANDS_MCP_TOKEN'}
    if len(clean_env) != len(os.environ):
        os.execvpe(sys.executable, [sys.executable, *sys.argv], clean_env)
    try:
        require(not (args.binding and args.current_shell))
        if args.current_shell:
            require(args.mode == 'stdio')
            binding, path, peer = current_shell()
            adapter(binding, required_remote=True, remote_path=path, expected_peer=peer)
            sys.exit(0)
        require(args.binding)
        directory(args.binding)
        if args.mode == 'stdio':
            adapter(args.binding)
        elif args.mode == 'control':
            control(args.binding, args.nonce)
        elif args.mode == 'shell':
            import runpy
            supervisor = runpy.run_path(str(Path(__file__).with_name('instance-shell.py')))
            sys.exit(supervisor['shell'](args.binding, Broker, serve, directory))
        elif args.mode == 'shell-command':
            require(len(args.command+trailing) == 1)
            sys.exit(shell_command(args.binding, (args.command+trailing)[0]))
        else:
            command = args.command+trailing
            if command and command[0] == '--': command.pop(0)
            argv = cli_args(args.binding, command); os.execvp(argv[0], argv)
    except Exception:
        print(ERROR, file=sys.stderr)
        sys.exit(1)
