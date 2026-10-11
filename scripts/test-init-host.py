#!/usr/bin/env python3
"""Isolated Linux integration tests for init-host.sh.

Run in WSL/Linux: python3 scripts/test-init-host.py --binary /absolute/openab-pty
The supplied binary must be the real upstream runtime. No pip packages needed.
Only the task-owned fixture HOME, stub commands and child runtime are changed.
"""

import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

INSTALLER = Path(__file__).resolve().with_name("init-host.sh")
HELPER_SPEC = importlib.util.spec_from_file_location('host_bundle', INSTALLER.parent / 'openab-runtime/host_bundle.py')
BUNDLE = importlib.util.module_from_spec(HELPER_SPEC)
HELPER_SPEC.loader.exec_module(BUNDLE)
PORT = 18090
STUB = r'''#!/usr/bin/env python3
import json,os,pathlib,signal,subprocess,sys
home=pathlib.Path(os.environ['HOME']); state=home/'stub-state'; state.mkdir(exist_ok=True)
args=sys.argv[1:]; name=pathlib.Path(sys.argv[0]).name
def flag(x): return (state/x).exists()
def pid():
    try: return int((state/'pid').read_text())
    except (OSError,ValueError): return 0
def alive():
    p=pid()
    if not p: return False
    try: os.kill(p,0); return True
    except ProcessLookupError: return False
if name=='systemctl':
    cmd=' '.join(args)
    if 'show-environment' in args or 'daemon-reload' in args: sys.exit(0)
    if 'cat' in args: sys.exit(1)
    if 'disable' in args:
        if pid():
            try: os.kill(pid(),signal.SIGTERM)
            except ProcessLookupError: pass
        sys.exit(0)
    if 'enable' in args:
        if flag('fail-service'): sys.exit(1)
        root=home/'.local/share/art-term/openab-pty'
        p=subprocess.Popen([str(root/'openab-pty'),'--config',str(root/'config.toml')],
            stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,
            start_new_session=True,close_fds=True)
        (state/'pid').write_text(str(p.pid)); sys.exit(0)
    if 'is-active' in args: sys.exit(0 if alive() else 1)
    sys.exit(2)
if name=='loginctl':
    if 'show-user' in args: print('no' if flag('linger-off') else 'yes'); sys.exit(0)
    if 'enable-linger' in args:
        (state/'linger-off').unlink(missing_ok=True)
        (state/'linger-enabled').write_text('yes'); sys.exit(0)
    sys.exit(2)
if name=='tailscale':
    if args[:2]==['status','--json']:
        print(json.dumps({'BackendState':'Running','Self':{'DNSName':'fixture.example.ts.net.'}})); sys.exit(0)
    if args[:2]==['ip','-4']: print('100.64.0.1'); sys.exit(0)
    if args[:2]==['serve','status']:
        mode=(state/'route').read_text() if flag('route') else 'absent'
        obj={'TCP':{},'Web':{},'AllowFunnel':{}}
        if mode in ('managed','conflict','funnel'):
            obj['TCP']={'8443':{'HTTPS':True}}
            obj['Web']={'fixture.example.ts.net:8443':{'Handlers':{'/':{'Proxy':'http://127.0.0.1:'+('18090' if mode!='conflict' else '12345')}}}}
        if mode=='funnel': obj['AllowFunnel']={'fixture.example.ts.net:8443':True}
        if mode=='unrelated': obj['Web']={'fixture.example.ts.net:443':{'Handlers':{'/other':{'Proxy':'http://127.0.0.1:9999'}}}}
        if mode=='managed-with-unrelated':
            obj['TCP']={'8443':{'HTTPS':True}}
            obj['Web']={'fixture.example.ts.net:8443':{'Handlers':{'/':{'Proxy':'http://127.0.0.1:18090'}}},'fixture.example.ts.net:443':{'Handlers':{'/other':{'Proxy':'http://127.0.0.1:9999'}}}}
        print(json.dumps(obj)); sys.exit(0)
    if args[:1]==['serve']:
        if flag('fail-serve'): sys.exit(1)
        old=(state/'route').read_text() if flag('route') else 'absent'
        (state/'route').write_text('managed-with-unrelated' if old=='unrelated' else 'managed')
        sys.exit(0)
    sys.exit(2)
if name=='sudo':
    os.execv(sys.executable,[sys.executable,str(pathlib.Path(sys.argv[0]).with_name(args[0])),*args[1:]])
if name=='uname': print('Darwin' if flag('bad-platform') else ('i686' if flag('bad-arch') and '-m' in args else ('x86_64' if '-m' in args else 'Linux'))); sys.exit(0)
if name=='id':
    if '-u' in args: print('0' if flag('root') else os.getuid()); sys.exit(0)
    if '-un' in args: print('fixture'); sys.exit(0)
sys.exit(2)
'''


def port_free():
    with socket.socket() as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            sock.bind(("127.0.0.1", PORT))
            return True
        except OSError:
            return False


class InstallerTests(unittest.TestCase):
    binary = None
    binary_hash = None

    def setUp(self):
        deadline = time.monotonic() + 65
        while not port_free() and time.monotonic() < deadline:
            time.sleep(.25)
        self.assertTrue(port_free(), f"127.0.0.1:{PORT} is occupied; refusing to touch another service")
        self.temp = tempfile.TemporaryDirectory(prefix="art-term-init-host-test-", dir=FIXTURE_OUTPUT)
        self.home = Path(self.temp.name)
        self.stub_dir = self.home / "bin"
        self.stub_dir.mkdir()
        for name in ("systemctl", "loginctl", "tailscale", "sudo", "uname", "id"):
            path = self.stub_dir / name
            path.write_text(STUB)
            path.chmod(0o700)
        self.state = self.home / "stub-state"
        self.state.mkdir()
        self.root = self.home / ".local/share/art-term/openab-pty"
        self.unit = self.home / ".config/systemd/user/art-term-openab-pty.service"
        self.env = dict(os.environ, HOME=str(self.home), PATH=str(self.stub_dir) + os.pathsep + os.environ["PATH"])

    def tearDown(self):
        pid_file = self.state / "pid"
        if pid_file.exists():
            pid = int(pid_file.read_text())
            try:
                os.killpg(pid, signal.SIGTERM)
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    try:
                        os.kill(pid, 0)
                    except ProcessLookupError:
                        break
                    time.sleep(.05)
                else:
                    os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        # The PTY shell can flush .bash_history just after the runtime exits.
        for attempt in range(20):
            try:
                self.temp.cleanup()
                break
            except OSError:
                if attempt == 19:
                    raise
                time.sleep(.1)
        deadline = time.monotonic() + 65
        while not port_free() and time.monotonic() < deadline:
            time.sleep(.25)
        self.assertTrue(port_free(), f"fixture runtime kept port {PORT} occupied")

    def run_installer(self, *args, success=True, env=None, timeout=50):
        cmd = ["bash", str(INSTALLER), *args]
        result = subprocess.run(cmd, env=env or self.env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                text=True, timeout=timeout)
        self.assertEqual(result.returncode == 0, success,
                         f"exit={result.returncode}; stdout={self.redact(result.stdout)}; stderr={self.redact(result.stderr)}")
        secret_file = self.root / "credential"
        if secret_file.exists():
            secret = secret_file.read_text().strip()
            self.assertNotIn(secret, result.stdout + result.stderr)
            self.assertNotIn(secret, "\0".join(cmd))
        return result

    def redact(self, data):
        secret_file = self.root / "credential"
        if secret_file.exists():
            data = data.replace(secret_file.read_text().strip(), "[REDACTED]")
        return data[-1500:]

    def install(self, *args, success=True):
        return self.run_installer("--binary", str(self.binary), "--sha256", self.binary_hash,
                                  *args, success=success)

    def request(self, path, token, method="GET", body=None, timeout=5):
        payload = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", data=payload,
              headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"}, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    def test_existing_install_can_enable_linger_without_rotation(self):
        self.install()
        credential = (self.root / "credential").read_bytes()
        pid = (self.state / "pid").read_bytes()
        (self.state / "linger-off").touch()
        self.run_installer("--enable-linger")
        self.assertTrue((self.state / "linger-enabled").exists())
        self.assertFalse((self.state / "linger-off").exists())
        self.assertEqual((self.root / "credential").read_bytes(), credential)
        self.assertEqual((self.state / "pid").read_bytes(), pid)

    def test_real_runtime_credential_and_pty(self):
        self.install()
        manifest = json.loads((self.root / "manifest.json").read_text())
        self.assertEqual(manifest["binary"], self.binary_hash)
        self.assertEqual(hashlib.sha256((self.root / "openab-pty").read_bytes()).hexdigest(), self.binary_hash)
        self.assertEqual(stat.S_IMODE((self.root / "credential").stat().st_mode), 0o600)
        credential = (self.root / "credential").read_text().strip()
        self.assertEqual(len(credential), 64)
        self.assertNotIn(credential, (self.root / "config.toml").read_text() + self.unit.read_text())
        pid = int((self.state / "pid").read_text())
        self.assertNotIn(credential.encode(), Path(f"/proc/{pid}/cmdline").read_bytes())
        code, _ = self.request("/admin/sessions", credential)
        self.assertEqual(code, 200)
        code, _ = self.request("/admin/sessions", "wrong")
        self.assertEqual(code, 401)
        code, payload = self.request("/admin/sessions", credential, "POST", {"name": "fixture"})
        self.assertEqual(code, 200)
        token = json.loads(payload)["token"]
        self.pty_smoke(token)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            code, body = self.request("/admin/sessions", credential)
            self.assertEqual(code, 200)
            sessions = json.loads(body)["sessions"]
            fixture = next((item for item in sessions if item["name"] == "fixture"), None)
            if fixture and fixture["alive"] and not fixture["attached"]:
                break
            time.sleep(.1)
        else:
            self.fail("detached session did not remain alive and unattached")

        # Upstream teardown allows 5 seconds for SIGTERM plus a bounded
        # SIGKILL/reap wait. A five-second HTTP deadline falsely classified
        # this normal grace period as a DELETE failure in the exploratory run.
        started = time.monotonic()
        code, payload = self.request("/admin/sessions/fixture", credential,
                                     "DELETE", timeout=15)
        elapsed = time.monotonic() - started
        self.assertEqual(code, 200)
        self.assertIsInstance(json.loads(payload), dict)
        code, payload = self.request("/admin/sessions", credential)
        self.assertEqual(code, 200)
        state = json.loads(payload)
        self.assertFalse(any(item["name"] == "fixture" for item in state["sessions"]))
        self.assertEqual(state["kill_domain"]["leaked_processes"], 0)
        print(f"DETACH_DELETE: status={code}, elapsed={elapsed:.3f}s, "
              "session_absent=true, leaked_processes=0", flush=True)

    def pty_smoke(self, token):
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        sock = socket.create_connection(("127.0.0.1", PORT), timeout=5)
        sock.settimeout(5)
        try:
            request = (f"GET /pty/fixture HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\n"
                       f"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                       f"Sec-WebSocket-Version: 13\r\nAuthorization: Bearer {token}\r\n\r\n")
            sock.sendall(request.encode())
            response = b""
            while b"\r\n\r\n" not in response:
                response += sock.recv(4096)
            header, pending = response.split(b"\r\n\r\n", 1)
            self.assertIn(b"101 Switching Protocols", header)
            self.ws_send(sock, 1, json.dumps({"v": 1, "type": "resize", "cols": 100, "rows": 35}).encode())
            self.ws_send(sock, 2, b"stty size\n")
            self.ws_send(sock, 2, b"printf '\\x4d\\x59\\x43\\x4c\\x49\\x5f\\x50\\x54\\x59\\x5f\\x53\\x4d\\x4f\\x4b\\x45\\x5f\\x4f\\x4b\\n'\n")
            found = False
            sized = False
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                opcode, data, pending = self.ws_recv(sock, pending)
                if opcode == 2:
                    found |= b"ARTTERM_PTY_SMOKE_OK" in data
                    sized |= b"35 100" in data
                    if found and sized:
                        break
            self.assertTrue(found, "real PTY did not return shell output")
            self.assertTrue(sized, "PTY resize was not reflected by stty size")
            self.ws_send(sock, 1, b'{"v":1,"type":"detach"}')
        finally:
            sock.close()

    @staticmethod
    def ws_send(sock, opcode, payload):
        mask = secrets.token_bytes(4)
        size = len(payload)
        head = bytes([0x80 | opcode, 0x80 | size]) if size < 126 else bytes([0x80 | opcode, 0x80 | 126]) + struct.pack("!H", size)
        sock.sendall(head + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(payload)))

    @staticmethod
    def ws_recv(sock, pending):
        def need(count):
            nonlocal pending
            while len(pending) < count:
                part = sock.recv(4096)
                if not part:
                    raise AssertionError("WebSocket closed before PTY output")
                pending += part
        need(2)
        opcode = pending[0] & 15
        size = pending[1] & 127
        offset = 2
        if size == 126:
            need(4)
            size = struct.unpack("!H", pending[2:4])[0]
            offset = 4
        elif size == 127:
            need(10)
            size = struct.unpack("!Q", pending[2:10])[0]
            offset = 10
        need(offset + size)
        data, pending = pending[offset:offset + size], pending[offset + size:]
        return opcode, data, pending

    def test_repeat_keeps_secret_and_pid(self):
        self.install()
        before = (self.root / "credential").read_bytes()
        pid = (self.state / "pid").read_text()
        result = self.run_installer("--show-info")
        self.assertIn('official openab-pty transport', result.stdout)
        self.assertNotIn('Current art-term v6 does not support this transport yet.', result.stdout)
        self.run_installer()
        self.assertEqual((self.root / "credential").read_bytes(), before)
        self.assertEqual((self.state / "pid").read_text(), pid)
        self.run_installer("--show-credential", success=False)

    def test_never_installed_inspection_friendly_without_creating_directories(self):
        for flag in ('--show-info', '--show-credential'):
            result = self.run_installer(flag, success=False)
            self.assertIn('Not installed yet.', result.stderr)
            self.assertNotIn('Bad file descriptor', result.stderr)
            self.assertNotIn('Errno 9', result.stderr)
            self.assertFalse((self.home / '.local').exists())
            self.assertFalse((self.home / '.config').exists())
        # A pre-existing empty parent does not justify creating unit directories
        # or a lock file merely to report that no installation exists.
        self.root.parent.mkdir(parents=True)
        result = self.run_installer('--show-info', success=False)
        self.assertIn('Not installed yet.', result.stderr)
        self.assertEqual(list(self.root.parent.iterdir()), [])
        self.assertFalse((self.home / '.config').exists())

    def test_sigkill_readiness_header_residue_does_not_wedge_bundle(self):
        self.install()
        before = (self.root / 'credential').read_bytes()
        runtime_pid = (self.state / 'pid').read_text()
        original_curl = shutil.which('curl', path=os.environ['PATH'])
        self.assertIsNotNone(original_curl)
        curl = self.stub_dir / 'curl'
        curl.write_text('''#!/usr/bin/env python3
import os,pathlib,signal,sys,time
state=pathlib.Path(os.environ['HOME'])/'stub-state'
if (state/'kill-readiness').exists():
    header=next((a[1:] for a in sys.argv[1:] if a.startswith('@')),None)
    if header:
        (state/'residue-path').write_text(header)
        deadline=time.monotonic()+2
        while not (state/'installer-pid').exists() and time.monotonic()<deadline: time.sleep(.01)
        os.kill(int((state/'installer-pid').read_text()),signal.SIGKILL)
        sys.exit(0)
os.execv(os.environ['FIXTURE_REAL_CURL'],[os.environ['FIXTURE_REAL_CURL'],*sys.argv[1:]])
''')
        curl.chmod(0o700)
        (self.state / 'kill-readiness').touch()
        env = dict(self.env, FIXTURE_REAL_CURL=original_curl)
        child = subprocess.Popen(['bash', str(INSTALLER), '--show-info'], env=env,
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        (self.state / 'installer-pid').write_text(str(child.pid))
        try:
            stdout, stderr = child.communicate(timeout=50)
        except BaseException:
            child.kill()
            child.communicate()
            raise
        self.assertEqual(child.returncode, -signal.SIGKILL)
        secret = before.strip()
        self.assertTrue(secret not in stdout + stderr, 'Credential leaked in interrupted probe output')
        residue = Path((self.state / 'residue-path').read_text())
        self.assertEqual(residue.parent, self.root.parent)
        self.assertEqual(stat.S_IMODE(residue.stat().st_mode), 0o600)
        self.assertTrue(residue.read_bytes() == b'Authorization: Bearer ' + secret + b'\n',
                        'Private probe header did not contain the controlled credential')
        self.assertEqual(set(p.name for p in self.root.iterdir()),
                         {'openab-pty', 'credential', 'config.toml', 'shell', 'service', 'manifest.json',
                          'preview.py', 'mcp_preview.py'})
        (self.state / 'kill-readiness').unlink()
        self.run_installer('--show-info', env=env)
        self.run_installer(env=env)
        self.assertTrue(residue.exists(), 'Test must retain the interruption residue through subsequent verification')
        self.assertTrue((self.root / 'credential').read_bytes() == before, 'Credential changed after interrupted inspection')
        self.assertEqual((self.state / 'pid').read_text(), runtime_pid)
        print('SIGKILL_HEADER_RESIDUE: parent_private600=true, bundle6=true, later_inspect/rerun_pass, runtime_pid_stable', flush=True)

    def test_upgrade_flags_and_pending_journal_fail_before_service_change(self):
        self.run_installer('--upgrade-runtime', success=False)
        self.install()
        pid = (self.state / 'pid').read_text()
        credential = (self.root / 'credential').read_bytes()
        self.run_installer('--upgrade-runtime', '--serve', success=False)
        self.run_installer('--recover-upgrade', '--show-info', success=False)
        journal = self.root.parent / '.upgrade-intent.json'
        journal.write_text('{}')
        self.run_installer(success=False)
        self.run_installer('--show-info', success=False)
        self.run_installer('--recover-upgrade', success=False)
        self.assertEqual((self.state / 'pid').read_text(), pid)
        self.assertEqual((self.root / 'credential').read_bytes(), credential)

    def test_real_runtime_upgrade_enrollment_continuity_with_fake_service_adapter(self):
        self.install()
        # A real official binary in the exact legacy managed format.
        # This tests enrollment continuity/actual restart, not real systemd gates.
        manifest = json.loads((self.root / 'manifest.json').read_text())
        manifest['format'] = 1
        for key, name in BUNDLE.LAUNCHER.items():
            (self.root / name).unlink()
            manifest.pop(key, None)
        BUNDLE.durable_json(self.root / 'manifest.json', manifest)
        before = {name: (self.root / name).read_bytes() for name in ('credential', 'config.toml', 'shell', 'service')}
        credential = (self.root / 'credential').read_text().strip()
        case = self
        class FixturePlatform(BUNDLE.Platform):
            def prerequisites(self):
                pass
            def ready(self, official):
                # Production Platform.ready intentionally targets fixed 18090.
                # This adapter uses the selected fixture port, retaining the
                # same authenticated-list and wrong-credential assertions.
                deadline = time.monotonic() + 15
                while True:
                    try:
                        code, payload = case.request('/admin/sessions', credential, timeout=3)
                        BUNDLE.require(code == 200, 'Fixture readiness authentication')
                        value = json.loads(payload)
                        BUNDLE.require(isinstance(value, dict) and isinstance(value.get('sessions'), list),
                                       'Fixture session list missing')
                        break
                    except (OSError, BUNDLE.Refusal):
                        BUNDLE.require(time.monotonic() < deadline, 'Fixture readiness timeout')
                        time.sleep(.2)
                BUNDLE.require(case.request('/admin/sessions', 'invalid-readiness-probe', timeout=3)[0] == 401,
                               'Fixture wrong credential response must be 401')
            def snapshot(self, m, allow_inactive=True):
                active = not port_free()
                BUNDLE.require(allow_inactive or active, 'Owned fixture inactive')
                return {'active': active, 'enabled': True, 'pid': int((case.state / 'pid').read_text()) if active else 0,
                        'cgroup': '/owned-fixture-only'}
            def ingress(self):
                return '{}'
            def stop(self, state):
                os.kill(state['pid'], signal.SIGTERM)
                deadline = time.monotonic() + 15
                while not port_free() and time.monotonic() < deadline:
                    time.sleep(.05)
                BUNDLE.require(port_free(), 'Owned runtime stop deadline')
            def start(self):
                p = subprocess.Popen([str(case.root / 'openab-pty'), '--config', str(case.root / 'config.toml')],
                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                    env=case.env, start_new_session=True, close_fds=True)
                (case.state / 'pid').write_text(str(p.pid))
                deadline = time.monotonic() + 5
                while port_free() and time.monotonic() < deadline:
                    time.sleep(.05)
        transaction = BUNDLE.Transaction(self.root, self.unit, FixturePlatform(self.root, self.unit))
        transaction.platform.ready(False)  # Format-1 official baseline has no custom identity fields.
        transaction.upgrade(str(self.binary), self.binary_hash)
        self.assertEqual(before, {name: (self.root / name).read_bytes() for name in before})
        self.assertEqual(json.loads((self.root / 'manifest.json').read_text())['format'], BUNDLE.OFFICIAL_FORMAT)
        code, state = self.request('/admin/sessions', credential)
        self.assertEqual(code, 200)
        self.assertIsInstance(json.loads(state if isinstance(state, bytes) else state).get('sessions', []), list)
        self.assertEqual(self.request('/admin/sessions', 'wrong')[0], 401)
        print('REAL_BINARY_UPGRADE: credential/config/shell/unit byte-identical; official sessions list; auth200/wrong401', flush=True)

    def test_service_failure_cleans_new_install(self):
        (self.state / "fail-service").touch()
        self.install(success=False)
        self.assertFalse(self.root.exists())
        self.assertFalse(self.unit.exists())

    def test_serve_failure_retains_runtime(self):
        (self.state / "fail-serve").touch()
        self.install("--serve", success=False)
        self.assertTrue(self.root.exists())
        self.assertEqual(self.request("/admin/sessions", (self.root / "credential").read_text().strip())[0], 200)

    def test_serve_adds_only_port_and_refuses_conflicts(self):
        (self.state / "route").write_text("unrelated")
        self.install("--serve")
        self.assertEqual((self.state / "route").read_text(), "managed-with-unrelated")
        self.run_installer("--serve")

    def test_existing_port_and_funnel_refusal(self):
        (self.state / "route").write_text("funnel")
        self.install("--serve", success=False)
        self.assertEqual((self.state / "route").read_text(), "funnel")
        self.assertTrue(self.root.exists())

    def test_occupied_listener_refused(self):
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", PORT))
            listener.listen()
            self.install(success=False)
        self.assertFalse(self.root.exists())

    def test_bad_binary_hash_and_arguments(self):
        self.install("--nonsense", success=False)
        self.run_installer("--binary", str(self.binary), "--sha256", "0" * 64, success=False)
        self.run_installer("--binary", "relative", "--sha256", self.binary_hash, success=False)
        link = self.home / "linked-binary"
        link.symlink_to(self.binary)
        self.run_installer("--binary", str(link), "--sha256", self.binary_hash, success=False)
        self.assertFalse(self.root.exists())

    def test_symlink_root_rejected(self):
        (self.home / ".local").symlink_to(self.home / "elsewhere")
        self.install(success=False)
        self.assertFalse((self.home / "elsewhere").exists())

    def test_incomplete_checkout_refused_before_mutation(self):
        fixture_scripts = self.home / "scripts"
        runtime = fixture_scripts / "openab-runtime"
        runtime.mkdir(parents=True)
        installer = fixture_scripts / "init-host.sh"
        shutil.copy(INSTALLER, installer)
        shutil.copy(INSTALLER.parent / "openab-runtime" / "host_bundle.py", runtime)
        result = subprocess.run(["bash", str(installer), "--show-info"], env=self.env,
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, timeout=50)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Incomplete checkout", result.stderr)
        self.assertFalse((self.home / ".local").exists())
        self.assertFalse((self.home / ".config").exists())

    def test_root_platform_and_missing_dependency_rejected(self):
        for flag in ("root", "bad-platform", "bad-arch"):
            (self.state / flag).touch()
            self.install(success=False)
            (self.state / flag).unlink()
        missing = self.stub_dir / "curl"
        missing.write_text("#!/bin/sh\nexit 0\n")
        missing.chmod(0o700)
        # A guaranteed missing dependency via a path with only necessary aliases.
        isolated = self.home / "missing-bin"
        isolated.mkdir()
        for name in ("bash", "uname", "id", "python3", "systemctl", "loginctl", "tailscale"):
            source = shutil.which(name, path=self.env["PATH"])
            (isolated / name).symlink_to(source)
        env = dict(self.env, PATH=str(isolated))
        self.install_with_env(env, success=False)

    def install_with_env(self, env, success):
        self.run_installer("--binary", str(self.binary), "--sha256", self.binary_hash,
                           success=success, env=env)


def main():
    global PORT, INSTALLER, STUB, FIXTURE_OUTPUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="absolute path to real upstream openab-pty")
    parser.add_argument("--port", type=int, default=18090, help="test-only installer projection port; production installer stays fixed")
    args = parser.parse_args()
    if sys.platform != "linux" or not args.binary.is_absolute() or not args.binary.is_file():
        parser.error("Linux/WSL and an existing absolute --binary path are required")
    if not 1 <= args.port <= 65535:
        parser.error("--port must be 1..65535")
    PORT = args.port
    if not port_free():
        parser.error(f"127.0.0.1:{PORT} is occupied; refusing to touch another listener")
    output = Path(__file__).resolve().parent.parent / 'target/linux-init-host'
    output.mkdir(parents=True, exist_ok=True)
    # Only this whitelist is copied, not a checkout. This test projection does
    # not make the production installer configurable or change its source.
    projection = tempfile.TemporaryDirectory(prefix='projection-', dir=output)
    FIXTURE_OUTPUT = Path(projection.name)
    scripts = FIXTURE_OUTPUT / 'scripts'
    scripts.mkdir()
    raw = INSTALLER.read_text()
    for needle, replacement in [('readonly PORT=18090', f'readonly PORT={PORT}'),
                                ('target="http://127.0.0.1:18090"', f'target="http://127.0.0.1:{PORT}"')]:
        if raw.count(needle) != 1:
            parser.error('installer projection contract changed; refusing broad replacement')
        raw = raw.replace(needle, replacement)
    INSTALLER = scripts / 'init-host.sh'
    INSTALLER.write_text(raw)
    shutil.copytree(Path(__file__).resolve().parent / 'openab-runtime', scripts / 'openab-runtime',
                    ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    STUB = STUB.replace('18090', str(PORT))
    InstallerTests.binary = args.binary.resolve()
    InstallerTests.binary_hash = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(InstallerTests)
    try:
        result = unittest.TextTestRunner(verbosity=2).run(suite)
    finally:
        projection.cleanup()
    print(f"RESULT: {result.testsRun} tests, {len(result.failures)} failures, {len(result.errors)} errors")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
