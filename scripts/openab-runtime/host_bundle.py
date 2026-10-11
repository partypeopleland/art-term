#!/usr/bin/env python3
"""Bounded, single-user Linux bundle upgrade. Called under installer flock FD 9.

No secret appears in argv or diagnostics. A hostile process with the same UID is
outside the trust boundary. SIGKILL recovery uses the durable journal, not traps.
"""
import argparse
import ctypes
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

PIN = 'b82a36620a76c84a67bc6a7ab316cbc60a30e1a9'
COMPAT = 'art-term-openab-v1'
UNIT = 'art-term-openab-pty.service'
ASSETS = Path(__file__).resolve().parent
HEX = re.compile(r'[a-f0-9]{64}')
FILES = {'binary': 'openab-pty', 'config': 'config.toml', 'shell': 'shell', 'unit': 'service'}
LAUNCHER = {'preview': 'preview.py', 'helper': 'mcp_preview.py'}
DESKTOP = 'desktop.env'
OFFICIAL_FORMAT = 4


def official():
    """Loaded lazily so test fixtures without network need not import it early."""
    import importlib.util
    spec = importlib.util.spec_from_file_location('official_artifact', ASSETS / 'official_artifact.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Refusal(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise Refusal(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def unique(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, 'Duplicate JSON key: ' + key)
        value[key] = item
    return value


def json_bytes(data):
    return json.loads(data, object_pairs_hook=unique)


def ancestry(path):
    for part in [path, *path.parents]:
        if part.exists() or part.is_symlink():
            require(not part.is_symlink(), 'Symlink in managed path: ' + str(part))


def private_stat(info, directory=False):
    require(info.st_uid == os.getuid() and not stat.S_IMODE(info.st_mode) & 0o077,
            'Managed file ownership/private permissions mismatch')
    require(stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode),
            'Managed path has unexpected type')


def directory(path, private=True):
    ancestry(path)
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if private:
            private_stat(info, True)
        else:
            require(info.st_uid == os.getuid() and stat.S_ISDIR(info.st_mode) and
                    not stat.S_IMODE(info.st_mode) & 0o022, 'Unsafe managed parent directory')
    except BaseException:
        os.close(fd)
        raise
    return fd


def read_at(fd, name):
    child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=fd)
    try:
        private_stat(os.fstat(child))
        with os.fdopen(child, 'rb', closefd=False) as stream:
            return stream.read()
    finally:
        os.close(child)


def private_read(path, parent_private=True):
    fd = directory(path.parent, parent_private)
    try:
        return read_at(fd, path.name)
    finally:
        os.close(fd)


def sync_dir(path):
    fd = directory(path)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def durable_json(path, value):
    parent_fd = directory(path.parent)
    temporary = '.journal-' + os.urandom(12).hex()
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                 0o600, dir_fd=parent_fd)
    try:
        with os.fdopen(fd, 'wb', closefd=False) as stream:
            stream.write((json.dumps(value, sort_keys=True, indent=2) + '\n').encode())
            stream.flush()
            os.fsync(fd)
        os.rename(temporary, path.name, src_dir_fd=parent_fd, dst_dir_fd=parent_fd)
        os.fsync(parent_fd)
    finally:
        os.close(fd)
        try:
            os.unlink(temporary, dir_fd=parent_fd)
        except FileNotFoundError:
            pass
        os.close(parent_fd)


def verify(root, unit):
    fd = directory(root)
    try:
        names = set(os.listdir(fd))
        require('manifest.json' in names, 'Missing managed manifest')
        data = {name: read_at(fd, name) for name in names}
    finally:
        os.close(fd)
    m = json_bytes(data['manifest.json'])
    require(isinstance(m, dict), 'Manifest must be a JSON object')
    require(m.get('format') in (1, 2, 3, OFFICIAL_FORMAT) and type(m.get('format')) is int and
            type(m.get('uid')) is int and m.get('uid') == os.getuid(), 'Manifest owner/format mismatch')
    expected = {'openab-pty', 'config.toml', 'shell', 'service', 'credential', 'manifest.json'}
    if m['format'] == 3:
        expected |= {'mcp_preview.py', 'desktop.env'}
    if m['format'] == OFFICIAL_FORMAT:
        expected |= {'preview.py', 'mcp_preview.py'}
        if DESKTOP in names or 'desktop_env' in m:
            expected.add(DESKTOP)
    require(names == expected, 'Unexpected/missing bundle artifact; inspect before upgrading')
    for key in [*FILES, 'credential_verifier']:
        require(isinstance(m.get(key), str) and HEX.fullmatch(m[key]), 'Missing/invalid manifest digest: ' + key)
    for key, name in FILES.items():
        require(digest(data[name]) == m[key], 'Managed digest mismatch: ' + name)
    if unit is not None:
        require(data['service'] == private_read(unit, False), 'Managed unit bytes mismatch')
    credential = data['credential'].strip()
    require(re.fullmatch(b'[a-f0-9]{64}', credential), 'Credential format mismatch')
    require(digest(credential) == m['credential_verifier'], 'Credential verifier mismatch')
    # This managed projection is deliberately limited; refuse duplicated keys.
    matches = re.findall(rb'^admin_credential_hash\s*=\s*"(sha256:[a-f0-9]{64})"\s*$',
                         data['config.toml'], re.M)
    require(matches == [('sha256:' + m['credential_verifier']).encode()], 'Config verifier mismatch')
    if m['format'] in (2, 3):
        require(m.get('upstream_pin') == PIN and m.get('compatibility_id') == COMPAT and
                isinstance(m.get('production_patch_sha256'), str) and
                HEX.fullmatch(m['production_patch_sha256']), 'Unsupported patched manifest')
        require(isinstance(m.get('provenance'), dict) and
                m['provenance'].get('kind') in ('source-built', 'provided-binary'), 'Missing binary provenance')
    if m['format'] == 3:
        for key, name in [('helper_sha256', 'mcp_preview.py'), ('desktop_env_sha256', 'desktop.env')]:
            require(isinstance(m.get(key), str) and HEX.fullmatch(m[key]) and digest(data[name]) == m[key],
                    'Desktop artifact digest mismatch: ' + name)
        require(m.get('desktop_compatibility') == 'art-term-desktop-mcp-v1' and
                isinstance(m.get('desktop_patch_sha256'), str) and HEX.fullmatch(m['desktop_patch_sha256']),
                'Unsupported desktop manifest')
    if m['format'] == OFFICIAL_FORMAT:
        require(m.get('upstream_pin') == PIN, 'Official manifest pin mismatch')
        require(isinstance(m.get('provenance'), dict) and
                m['provenance'].get('kind') in ('official-ghcr', 'provided-binary'),
                'Missing official binary provenance')
        for key, name in LAUNCHER.items():
            require(isinstance(m.get(key), str) and HEX.fullmatch(m[key]) and digest(data[name]) == m[key],
                    'Launcher digest mismatch: ' + name)
        if DESKTOP in names:
            require(isinstance(m.get('desktop_env'), str) and HEX.fullmatch(m['desktop_env']) and
                    digest(data[DESKTOP]) == m['desktop_env'], 'Desktop config digest mismatch')
    return m, data


def run(args, timeout=15, cwd=None):
    result = subprocess.run(args, cwd=cwd, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    require(result.returncode == 0, 'Command failed: ' + str(args[0]) + ' ' + str(args[1]))
    return result.stdout


def check_binary(binary, config=None):
    """Official binary checks; generated credential material stays in memory."""
    result = subprocess.run([str(binary), '--generate-admin-credential'], stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
    require(result.returncode == 0 and len(result.stdout) <= 4096,
            'Official binary credential generation failed')
    credential = re.search(rb'^credential: ([a-f0-9]{64})$', result.stdout, re.M)
    verifier = re.search(rb'^admin_credential_hash = "(sha256:[a-f0-9]{64})"$', result.stdout, re.M)
    require(credential is not None and verifier is not None and
            digest(credential.group(1)) == verifier.group(1)[len(b'sha256:'):].decode(),
            'Official binary credential output mismatch')
    if config:
        run([str(binary), '--validate-projection', str(config)], 10)


def supplied_bytes(supplied, expected):
    source = Path(supplied)
    ancestry(source)
    require(source.is_absolute() and source.is_file() and not source.is_symlink(), 'Invalid supplied binary')
    fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        require(stat.S_ISREG(os.fstat(fd).st_mode), 'Supplied binary must be regular')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            data = stream.read()
    finally:
        os.close(fd)
    require(HEX.fullmatch(expected) and digest(data) == expected, 'Supplied binary SHA256 mismatch')
    return data


def build(destination, supplied='', expected=''):
    """Assemble a format-4 official bundle candidate in an empty scratch dir."""
    fd = directory(destination)
    os.close(fd)
    target = destination / 'openab-pty'
    if supplied:
        data = supplied_bytes(supplied, expected)
        provenance = {'kind': 'provided-binary', 'owner_supplied_sha256': expected,
                      'source_attestation': False}
    else:
        artifact = official()
        data = artifact.fetch()
        provenance = artifact.provenance()
    target.write_bytes(data)
    target.chmod(0o700)
    check_binary(target)
    for name in LAUNCHER.values():
        path = destination / name
        path.write_bytes((ASSETS / name).read_bytes())
        path.chmod(0o700)
    provenance['actual_binary_sha256'] = digest(data)
    durable_json(destination / 'build-provenance.json', provenance)
    return provenance


def manifest(root):
    p = json_bytes(private_read(root / 'build-provenance.json'))
    require(isinstance(p, dict) and p.get('kind') in ('official-ghcr', 'provided-binary'),
            'Unsupported build provenance for the official bundle')
    value = {'format': OFFICIAL_FORMAT, 'uid': os.getuid(), 'upstream_pin': PIN, 'provenance': p}
    for key, name in {**FILES, **LAUNCHER}.items():
        value[key] = digest(private_read(root / name))
    value['credential_verifier'] = digest(private_read(root / 'credential').strip())
    if (root / DESKTOP).exists():
        value['desktop_env'] = digest(private_read(root / DESKTOP))
    durable_json(root / 'manifest.json', value)
    (root / 'build-provenance.json').unlink()
    sync_dir(root)


def exchange(parent, left, right):
    fd = directory(parent)
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        rename = getattr(libc, 'renameat2', None)
        require(rename is not None, 'renameat2 unavailable; no upgrade attempted')
        rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        rename.restype = ctypes.c_int
        if rename(fd, os.fsencode(left), fd, os.fsencode(right), 2) != 0:
            raise OSError(ctypes.get_errno(), 'Atomic bundle exchange failed')
        os.fsync(fd)
    finally:
        os.close(fd)


def probe_exchange(parent):
    left = Path(tempfile.mkdtemp(prefix='.exchange-probe-', dir=parent))
    right = Path(tempfile.mkdtemp(prefix='.exchange-probe-', dir=parent))
    try:
        exchange(parent, left.name, right.name)
    finally:
        left.rmdir()
        right.rmdir()
        sync_dir(parent)


class Platform:
    """Real systemd/Tailnet adapter. Tests inject a separate fake adapter."""
    def __init__(self, root, unit):
        self.root, self.unit = root, unit

    def ingress(self):
        status = json_bytes(run(['tailscale', 'status', '--json'], 10))
        require(isinstance(status, dict) and status.get('BackendState') == 'Running', 'Tailnet unavailable; ingress unproven')
        value = json_bytes(run(['tailscale', 'serve', 'status', '--json'], 10))
        require(isinstance(value, dict), 'Invalid effective Serve/Funnel JSON; ingress unproven')
        return json.dumps(value, sort_keys=True, separators=(',', ':'))

    def prerequisites(self):
        runtime = Path(os.environ.get('XDG_RUNTIME_DIR', ''))
        require(str(runtime) == '/run/user/' + str(os.getuid()), 'Invalid XDG_RUNTIME_DIR; log in normally')
        private_stat(runtime.lstat(), True)
        bus = (runtime / 'bus').lstat()
        require(stat.S_ISSOCK(bus.st_mode) and bus.st_uid == os.getuid(), 'User bus missing/owner mismatch')
        run(['systemctl', '--user', 'show-environment'], 10)
        user = run(['id', '-un']).decode().strip()
        require(run(['loginctl', 'show-user', user, '-p', 'Linger', '--value']).strip() == b'yes', 'Linger is not enabled')

    def snapshot(self, m, allow_inactive=True):
        properties = ['LoadState', 'ActiveState', 'SubState', 'UnitFileState', 'FragmentPath',
                      'DropInPaths', 'Restart', 'KillMode', 'MainPID', 'ControlGroup', 'NeedDaemonReload']
        output = run(['systemctl', '--user', 'show', UNIT, '--no-pager',
                      '--property=' + ','.join(properties)], 10).decode()
        s = dict(line.split('=', 1) for line in output.splitlines() if '=' in line)
        require(s.get('LoadState') == 'loaded' and s.get('UnitFileState') == 'enabled' and
                s.get('FragmentPath') == str(self.unit) and not s.get('DropInPaths') and
                s.get('Restart') == 'on-failure' and s.get('KillMode') == 'control-group' and
                s.get('NeedDaemonReload') == 'no', 'Unsupported/mismatched loaded user unit')
        loaded = run(['systemctl', '--user', 'cat', UNIT, '--no-pager'], 10)
        prefix = ('# ' + str(self.unit) + '\n').encode()
        require(loaded.startswith(prefix) and loaded[len(prefix):] == private_read(self.unit, False),
                'Loaded unit content mismatch')
        expected_exec = ('ExecStart=' + str(self.root / 'openab-pty') + ' --config ' + str(self.root / 'config.toml')).encode()
        require(private_read(self.unit, False).splitlines().count(expected_exec) == 1,
                'Managed unit executable/config projection mismatch')
        state = (s.get('ActiveState'), s.get('SubState'))
        require(state == ('active', 'running') or (allow_inactive and state == ('inactive', 'dead')),
                'Unsupported service state (failed/masked/transitioning refused)')
        pid = int(s.get('MainPID', '-1'))
        group = s.get('ControlGroup', '')
        if group:
            require(group.startswith('/user.slice/user-' + str(os.getuid()) + '.slice/user@' + str(os.getuid()) + '.service/')
                    and group.endswith('/' + UNIT) and '..' not in group, 'Unexpected user-unit cgroup')
        members = self.members(group)
        if state == ('active', 'running'):
            require(pid > 0 and group and pid in members, 'MainPID/cgroup mismatch')
            proc = Path('/proc') / str(pid)
            require(proc.stat().st_uid == os.getuid() and os.readlink(proc / 'exe') == str(self.root / 'openab-pty'),
                    'MainPID owner/executable path mismatch')
            require(digest((proc / 'exe').read_bytes()) == m['binary'], 'MainPID executable digest mismatch')
            cgroups = (proc / 'cgroup').read_text().splitlines()
            require('0::' + group in cgroups, 'MainPID cgroup identity mismatch')
        else:
            require(pid == 0 and not members, 'Inactive service has stale processes')
        # Refuse other live runtimes at this managed path; never kill them.
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    if entry.stat().st_uid == os.getuid() and os.readlink(entry / 'exe') == str(self.root / 'openab-pty'):
                        require(int(entry.name) == pid, 'Orphan managed runtime detected')
                except (FileNotFoundError, ProcessLookupError, PermissionError):
                    continue
        return {'active': state[0] == 'active', 'enabled': True, 'pid': pid, 'cgroup': group}

    def members(self, group):
        if not group:
            return set()
        path = Path('/sys/fs/cgroup') / group.lstrip('/')
        if not path.exists():
            return set()
        result = set()
        for child in [path, *path.rglob('*')]:
            if child.is_dir():
                try:
                    result.update(int(p) for p in (child / 'cgroup.procs').read_text().split())
                except FileNotFoundError:
                    pass
        return result

    def stop(self, old):
        run(['systemctl', '--user', 'stop', UNIT], 25)
        require(not self.members(old['cgroup']), 'Service cgroup not empty after stop; manual inspection required')
        require(run(['systemctl', '--user', 'show', UNIT, '-p', 'MainPID', '--value']).strip() == b'0',
                'MainPID not zero after bounded stop')

    def start(self):
        run(['systemctl', '--user', 'start', UNIT], 25)

    def reload(self):
        run(['systemctl', '--user', 'daemon-reload'], 10)

    def ready(self, official):
        credential = private_read(self.root / 'credential').strip().decode()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        url = 'http://127.0.0.1:18090/admin/sessions'
        deadline = time.monotonic() + 15
        while True:
            try:
                request = urllib.request.Request(url, headers={'Authorization': 'Bearer ' + credential})
                with opener.open(request, timeout=3) as response:
                    value = json_bytes(response.read())
                # Format 1 predates the custom identity extension. Recovery
                # relies on the verified bundle/PID and authenticated API,
                # without imposing fields that its runtime never supplied.
                require(isinstance(value, dict) and isinstance(value.get('sessions'), list),
                        'Runtime session list missing')
                break
            except (OSError, urllib.error.HTTPError):
                require(time.monotonic() < deadline, 'Runtime readiness timeout')
                time.sleep(.2)
        try:
            opener.open(urllib.request.Request(url, headers={'Authorization': 'Bearer invalid-readiness-probe'}), timeout=3)
        except urllib.error.HTTPError as error:
            require(error.code == 401, 'Wrong credential response must be 401')
        else:
            raise Refusal('Wrong credential was accepted')


class Transaction:
    def __init__(self, root, unit, platform=None):
        self.root, self.unit = root, unit
        self.parent = root.parent
        self.backup = self.parent / '.upgrade-backup'
        self.journal = self.parent / '.upgrade-intent.json'
        self.platform = platform or Platform(root, unit)

    def pending(self):
        for name in ('.upgrade-intent.json', '.desktop-upgrade-intent.json'):
            path = self.parent / name
            require(not path.exists() and not path.is_symlink(),
                    'Upgrade intent exists; use the matching recovery flag. Journal: ' + str(path))

    def phase(self, value, name):
        value['phase'] = name
        durable_json(self.journal, value)

    def pairs(self, value):
        a, ad = verify(self.root, self.unit)
        b, bd = verify(self.backup, self.unit)
        ah, bh = digest(ad['manifest.json']), digest(bd['manifest.json'])
        require((ah, bh) in [(value['previous_manifest'], value['candidate_manifest']),
                              (value['candidate_manifest'], value['previous_manifest'])],
                'Unknown/corrupt interrupted bundle pair; manual recovery required')
        require({a['binary'], b['binary']} == {value['previous_binary'], value['candidate_binary']},
                'Interrupted binary identities mismatch')
        for name in ['credential', 'config.toml', 'shell', 'service']:
            require(ad[name] == bd[name], 'Interrupted enrollment differs: ' + name)
        if DESKTOP in ad and DESKTOP in bd:
            require(ad[DESKTOP] == bd[DESKTOP], 'Interrupted desktop config differs')
        return a, ah

    def recover(self):
        require(not (self.parent / '.desktop-upgrade-intent.json').exists() and
                not (self.parent / '.desktop-upgrade-intent.json').is_symlink(), 'Desktop recovery is pending')
        self.platform.prerequisites()
        value = json_bytes(private_read(self.journal))
        require(isinstance(value, dict), 'Upgrade journal must be a JSON object')
        require(type(value.get('version')) is int and value.get('version') == 1 and
                type(value.get('uid')) is int and value.get('uid') == os.getuid() and
                value.get('root') == str(self.root) and value.get('unit') == str(self.unit) and
                value.get('backup') == str(self.backup) and value.get('phase') in
                ('PREPARED', 'STOPPED', 'SWAPPED', 'VERIFIED', 'RECOVERING'), 'Unknown upgrade journal; manual recovery required')
        require(value.get('old_enabled') is True and type(value.get('old_active')) is bool, 'Invalid recorded service state')
        require(isinstance(value.get('ingress'), str) and isinstance(json_bytes(value['ingress']), dict),
                'Missing/invalid recorded effective ingress')
        for key in ['previous_manifest', 'candidate_manifest', 'previous_binary', 'candidate_binary']:
            require(isinstance(value.get(key), str) and HEX.fullmatch(value[key]), 'Invalid journal identity: ' + key)
        m, root_hash = self.pairs(value)
        state = self.platform.snapshot(m)
        self.phase(value, 'RECOVERING')
        if state['active']:
            self.platform.stop(state)
        self.pairs(value)
        if root_hash == value['candidate_manifest']:
            exchange(self.parent, self.root.name, self.backup.name)
        old, _ = verify(self.root, self.unit)
        require(old['binary'] == value['previous_binary'], 'Recovery did not restore original binary')
        if value['old_active']:
            self.platform.start()
            self.platform.ready(old['format'] == OFFICIAL_FORMAT)
            self.platform.snapshot(old, False)
        else:
            require(not self.platform.snapshot(old)['active'], 'Original inactive state was not restored')
        preserved = self.platform.ingress() == value['ingress']
        value['phase'] = 'RECOVERED'
        value['ingress_preserved'] = preserved
        durable_json(self.parent / '.upgrade-last.json', value)
        self.journal.unlink()
        sync_dir(self.parent)
        require(preserved, 'Old bundle restored; ingress changed/unproven. No Tailnet routes were modified; inspect manually')
        print('Original complete bundle and service state recovered; sessions cannot be restored. Private backup retained: ' + str(self.backup))

    def upgrade(self, supplied='', expected=''):
        self.pending()
        directory_fd = directory(self.parent)
        os.close(directory_fd)
        old, data = verify(self.root, self.unit)
        if supplied:
            supplied_bytes(supplied, expected)
            candidate = expected
        else:
            candidate = official().BINARY_SHA256
        if old['format'] == OFFICIAL_FORMAT and candidate == old['binary']:
            check_binary(self.root / 'openab-pty', self.root / 'config.toml')
            self.platform.prerequisites()
            self.platform.snapshot(old)
            print('Already at the pinned official runtime; no download or restart.')
            return
        require(not self.backup.exists() and not self.backup.is_symlink(),
                'Prior private backup remains; inspect and explicitly remove after acceptance: ' + str(self.backup))
        self.platform.prerequisites()
        state = self.platform.snapshot(old)
        ingress = self.platform.ingress()
        probe_exchange(self.parent)
        self.backup.mkdir(mode=0o700)
        journal_written = False
        try:
            carry = ['credential', 'config.toml', 'shell', 'service']
            if DESKTOP in data:
                carry.append(DESKTOP)
            for name in carry:
                (self.backup / name).write_bytes(data[name])
                (self.backup / name).chmod(0o700 if name == 'shell' else 0o600)
            build(self.backup, supplied, expected)
            check_binary(self.backup / 'openab-pty', self.backup / 'config.toml')
            manifest(self.backup)
            new, nd = verify(self.backup, self.unit)
            for path in self.backup.iterdir():
                fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
                try:
                    os.fsync(fd)
                finally:
                    os.close(fd)
            sync_dir(self.backup)
            sync_dir(self.parent)
            again, again_data = verify(self.root, self.unit)
            require(again_data == data and self.platform.snapshot(again) == state, 'Installation/service changed while building')
            require(self.platform.ingress() == ingress, 'Ingress changed while building; no service stopped')
            value = {'version': 1, 'uid': os.getuid(), 'root': str(self.root), 'unit': str(self.unit),
                     'backup': str(self.backup), 'previous_binary': old['binary'], 'candidate_binary': new['binary'],
                     'previous_manifest': digest(data['manifest.json']), 'candidate_manifest': digest(nd['manifest.json']),
                     'old_enabled': state['enabled'], 'old_active': state['active'], 'ingress': ingress}
            self.phase(value, 'PREPARED')
            journal_written = True
            print('Explicit --upgrade-runtime acknowledged: all existing PTYs terminate before bundle exchange.', flush=True)
            # Immediate revalidation after durable intent, before any stop.
            require(verify(self.root, self.unit)[1] == data and self.platform.snapshot(old) == state,
                    'Pre-stop installation/service identity changed')
            if state['active']:
                self.platform.stop(state)
            self.phase(value, 'STOPPED')
            self.pairs(value)
            require(not self.platform.snapshot(old)['active'], 'Service unexpectedly restarted before swap')
            exchange(self.parent, self.root.name, self.backup.name)
            self.phase(value, 'SWAPPED')
            verify(self.root, self.unit)
            if state['active']:
                self.platform.start()
                self.platform.ready(True)
                self.platform.snapshot(new, False)
            else:
                require(not self.platform.snapshot(new)['active'], 'Inactive service unexpectedly started')
            require(self.platform.ingress() == ingress, 'Ingress preservation unproven; no Tailnet mutation performed')
            self.phase(value, 'VERIFIED')
            durable_json(self.parent / '.upgrade-last.json', value)
            self.journal.unlink()
            sync_dir(self.parent)
            print('Official bundle installed; enrollment bytes and effective ingress preserved. Private rollback bundle: ' + str(self.backup))
        except BaseException:
            if journal_written:
                try:
                    self.recover()
                except BaseException as recovery_error:
                    record = self.journal if self.journal.exists() else self.parent / '.upgrade-last.json'
                    print('Recovery failed or ingress unproven: ' + str(recovery_error) + '. Recovery record: ' + str(record), file=sys.stderr)
            else:
                # Candidate is exclusively ours; running baseline was never stopped.
                shutil.rmtree(self.backup)
                sync_dir(self.parent)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['verify', 'build', 'manifest', 'upgrade', 'recover', 'pending',
                                        'desktop-enable', 'desktop-recover', 'desktop-rollback', 'desktop-stage'])
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--unit', type=Path)
    parser.add_argument('--binary', default='')
    parser.add_argument('--sha256', default='')
    parser.add_argument('--desktop-config', type=Path)
    args = parser.parse_args()
    def interrupted(signum, frame):
        raise Refusal('Installer interrupted; durable journal governs recovery')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        lock = os.fstat(9)
        private_stat(lock)
        lock_path = args.root.parent / '.installer.lock'
        ancestry(lock_path)
        expected_lock = lock_path.lstat()
        require((lock.st_dev, lock.st_ino) == (expected_lock.st_dev, expected_lock.st_ino),
                'Installer lock identity mismatch')
        fcntl.flock(9, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.action == 'build':
            build(args.root, args.binary, args.sha256)
        elif args.action == 'manifest':
            manifest(args.root)
        elif args.action == 'desktop-stage':
            import desktop_bundle
            desktop_bundle.stage(sys.modules[__name__], args.root, args.desktop_config)
        else:
            require(args.unit is not None, '--unit is required')
            transaction = Transaction(args.root, args.unit)
            if args.action.startswith('desktop-'):
                import desktop_bundle
                tx = desktop_bundle.DesktopTransaction(sys.modules[__name__], args.root, args.unit)
                if args.action == 'desktop-enable':
                    tx.enable(args.desktop_config, args.binary, args.sha256)
                elif args.action == 'desktop-recover':
                    tx.recover()
                else:
                    tx.rollback()
            elif args.action == 'verify':
                verify(args.root, args.unit)
            elif args.action == 'pending':
                transaction.pending()
            elif args.action == 'recover':
                transaction.recover()
            else:
                transaction.upgrade(args.binary, args.sha256)
    except (Refusal, OSError, ValueError, subprocess.SubprocessError) as error:
        print('ERROR: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
