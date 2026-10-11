#!/usr/bin/env python3
"""Linux-only isolated bundle/journal tests. Real rename/fsync, fake systemd.

No real user unit or Tailnet is changed. SIGKILL tests fork only owned children.
Actual runtime authentication/PTY tests are in test-init-host.py.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

SPEC = importlib.util.spec_from_file_location('host_bundle', Path(__file__).parent / 'openab-runtime/host_bundle.py')
B = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(B)


class FakePlatform:
    def __init__(self, state):
        self.state = state
        state.write_text('true')

    def active(self):
        return json.loads(self.state.read_text())

    def prerequisites(self):
        pass

    def snapshot(self, manifest, allow_inactive=True):
        if not allow_inactive:
            B.require(self.active(), 'Fake service inactive')
        return {'active': self.active(), 'enabled': True, 'pid': 777 if self.active() else 0, 'cgroup': '/fixture-owned'}

    def stop(self, state):
        self.state.write_text('false')

    def start(self):
        self.state.write_text('true')

    def ready(self, official):
        B.require(self.active(), 'Not ready')

    def ingress(self):
        return '{"TCP":{},"Web":{}}'


class UpgradeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='art-term-upgrade-test-')
        self.parent = Path(self.temp.name)
        self.root = self.parent / 'openab-pty'
        self.root.mkdir(mode=0o700)
        self.unit = self.parent / B.UNIT
        self.unit.write_bytes(('[Service]\nRestart=on-failure\nExecStart=' + str(self.root / 'openab-pty') +
                               ' --config ' + str(self.root / 'config.toml') + '\n').encode())
        self.unit.chmod(0o600)
        self.raw = b'a' * 64
        verifier = B.digest(self.raw)
        files = {'openab-pty': b'old-binary', 'credential': self.raw + b'\n',
                 'config.toml': ('[pty]\nadmin_credential_hash = "sha256:' + verifier + '"\n').encode(),
                 'shell': b'#!/bin/bash\nexec /bin/bash -i\n', 'service': self.unit.read_bytes()}
        for name, data in files.items():
            (self.root / name).write_bytes(data)
            (self.root / name).chmod(0o700 if name in ('shell', 'openab-pty') else 0o600)
        self.old = {'format': 1, 'uid': os.getuid(), 'credential_verifier': verifier}
        self.old.update({key: B.digest(files[name]) for key, name in B.FILES.items()})
        B.durable_json(self.root / 'manifest.json', self.old)
        self.platform = FakePlatform(self.parent / 'fake-state')
        self.tx = B.Transaction(self.root, self.unit, self.platform)
        self.patches = [patch.object(B, 'check_binary', return_value=None),
                        patch.object(B, 'build', side_effect=self.build)]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in reversed(self.patches):
            p.stop()
        self.temp.cleanup()

    def build(self, destination, supplied='', expected=''):
        (destination / 'openab-pty').write_bytes(b'new-binary')
        (destination / 'openab-pty').chmod(0o700)
        for name in B.LAUNCHER.values():
            (destination / name).write_bytes(b'# fixture ' + name.encode())
            (destination / name).chmod(0o700)
        B.durable_json(destination / 'build-provenance.json',
                       {'kind': 'provided-binary', 'actual_binary_sha256': B.digest(b'new-binary'), 'source_attestation': False})

    def test_explicit_upgrade_preserves_bytes_and_state(self):
        before = {name: (self.root / name).read_bytes() for name in ('credential', 'config.toml', 'shell', 'service')}
        self.tx.upgrade()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'new-binary')
        self.assertEqual((self.tx.backup / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())
        self.assertFalse(self.tx.journal.exists())
        self.assertEqual(before, {name: (self.root / name).read_bytes() for name in before})

    def test_inactive_remains_inactive(self):
        self.platform.state.write_text('false')
        self.tx.upgrade()
        self.assertFalse(self.platform.active())

    def test_format3_migration_preserves_environment_unit_and_legacy_desktop_record(self):
        env = b'ART_TERM_DESKTOP_MCP_URL=http://192.0.2.10/mcp\nHANDS_MCP_TOKEN=fixture-token\n'
        helper = b'legacy-helper'
        unit = self.unit.read_bytes() + ('EnvironmentFile=' + str(self.root / 'desktop.env') + '\n').encode()
        self.unit.write_bytes(unit)
        for name, raw in [('desktop.env', env), ('mcp_preview.py', helper), ('service', unit)]:
            (self.root / name).write_bytes(raw)
            (self.root / name).chmod(0o600)
        old = dict(self.old, format=3, upstream_pin=B.PIN, compatibility_id=B.COMPAT,
                   production_patch_sha256='b' * 64, desktop_patch_sha256='c' * 64,
                   desktop_compatibility='art-term-desktop-mcp-v1', provenance={'kind': 'source-built'},
                   helper_sha256=B.digest(helper), desktop_env_sha256=B.digest(env), unit=B.digest(unit))
        B.durable_json(self.root / 'manifest.json', old)
        last = self.parent / '.desktop-upgrade-last.json'
        B.durable_json(last, {'phase': 'VERIFIED', 'fixture': 'retained'})
        previous_last = last.read_bytes()
        self.tx.upgrade()
        m, data = B.verify(self.root, self.unit)
        self.assertEqual(m['format'], B.OFFICIAL_FORMAT)
        self.assertEqual(data['desktop.env'], env)
        self.assertEqual(data['service'], unit)
        self.assertEqual(last.read_bytes(), previous_last)
        self.assertEqual(B.verify(self.tx.backup, None)[0]['format'], 3)

    def test_current_noop_and_downgrade_refusal(self):
        self.tx.upgrade()
        m, _ = B.verify(self.root, self.unit)
        self.assertEqual(m['format'], B.OFFICIAL_FORMAT)
        official = SimpleNamespace(BINARY_SHA256=B.digest(b'new-binary'))
        with patch.object(B, 'official', return_value=official), \
             patch.object(B, 'build', side_effect=AssertionError('must not build')), \
             patch.object(self.platform, 'stop', side_effect=AssertionError('must not stop')):
            self.tx.upgrade()
        m['provenance'] = {'kind': 'source-built'}
        B.durable_json(self.root / 'manifest.json', m)
        with self.assertRaises(B.Refusal):
            self.tx.upgrade()

    def test_build_failure_before_stop(self):
        with patch.object(B, 'build', side_effect=B.Refusal('build failed')):
            with self.assertRaises(B.Refusal):
                self.tx.upgrade()
        self.assertTrue(self.platform.active())
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertFalse(self.tx.backup.exists())

    def test_readiness_failure_exact_rollback(self):
        original = self.platform.ready
        def ready(patched):
            if patched:
                raise B.Refusal('injected readiness failure')
            original(patched)
        with patch.object(self.platform, 'ready', side_effect=ready):
            with self.assertRaises(B.Refusal):
                self.tx.upgrade()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())
        self.assertFalse(self.tx.journal.exists())

    def test_stop_failure_retains_original(self):
        original = self.platform.stop
        calls = 0
        def stop(state):
            nonlocal calls
            calls += 1
            if calls == 1:
                raise B.Refusal('stop failed')
            original(state)
        with patch.object(self.platform, 'stop', side_effect=stop):
            with self.assertRaises(B.Refusal):
                self.tx.upgrade()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())

    def test_start_failure_restores_original(self):
        original = self.platform.start
        calls = 0
        def start():
            nonlocal calls
            calls += 1
            if calls == 1:
                raise B.Refusal('start failed')
            original()
        with patch.object(self.platform, 'start', side_effect=start):
            with self.assertRaises(B.Refusal):
                self.tx.upgrade()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())

    def test_manifest_future_duplicate_missing_uid_permissions_symlink(self):
        original = (self.root / 'manifest.json').read_bytes()
        cases = [b'{"format":1,"format":1}', json.dumps(dict(self.old, format=3)).encode(),
                 json.dumps(dict(self.old, uid=-1)).encode(), json.dumps({'format': 1, 'uid': os.getuid()}).encode()]
        for value in cases:
            (self.root / 'manifest.json').write_bytes(value)
            with self.assertRaises((B.Refusal, KeyError)):
                B.verify(self.root, self.unit)
        (self.root / 'manifest.json').write_bytes(original)
        (self.root / 'credential').chmod(0o644)
        with self.assertRaises(B.Refusal):
            B.verify(self.root, self.unit)
        (self.root / 'credential').chmod(0o600)
        (self.root / 'credential').unlink()
        (self.root / 'credential').symlink_to(self.unit)
        with self.assertRaises(OSError):
            B.verify(self.root, self.unit)

    def test_config_verifier_and_unexpected_mutable_artifact_refused(self):
        (self.root / 'unrecorded-state').write_text('unknown')
        with self.assertRaises(B.Refusal):
            B.verify(self.root, self.unit)
        (self.root / 'unrecorded-state').unlink()
        m = dict(self.old)
        changed = b'[pty]\nadmin_credential_hash = "sha256:' + b'0' * 64 + b'"\n'
        (self.root / 'config.toml').write_bytes(changed)
        m['config'] = B.digest(changed)
        B.durable_json(self.root / 'manifest.json', m)
        with self.assertRaises(B.Refusal):
            B.verify(self.root, self.unit)

    def test_prior_backup_refused_before_stop(self):
        self.tx.backup.mkdir(mode=0o700)
        with self.assertRaises(B.Refusal):
            self.tx.upgrade()
        self.assertTrue(self.platform.active())

    def crash(self, phase, exchange_window=False):
        sys.stdout.flush()
        sys.stderr.flush()
        child = os.fork()
        if child == 0:
            original = self.tx.phase
            def stage(value, name):
                original(value, name)
                if name == phase:
                    os.kill(os.getpid(), signal.SIGKILL)
            if exchange_window:
                original_exchange = B.exchange
                def interrupted(parent, left, right):
                    original_exchange(parent, left, right)
                    if left == self.root.name:
                        os.kill(os.getpid(), signal.SIGKILL)
                B.exchange = interrupted
            else:
                self.tx.phase = stage
            self.tx.upgrade()
            os._exit(23)
        _, status = os.waitpid(child, 0)
        self.assertTrue(os.WIFSIGNALED(status))
        self.assertEqual(os.WTERMSIG(status), signal.SIGKILL)
        with self.assertRaises(B.Refusal):
            self.tx.pending()
        return json.loads(self.tx.journal.read_text())

    def test_sigkill_prepared_recover(self):
        self.crash('PREPARED')
        self.tx.recover()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())

    def test_sigkill_stopped_recover(self):
        self.crash('STOPPED')
        self.tx.recover()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())

    def test_sigkill_exchange_before_journal_recover(self):
        self.crash('none', True)
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'new-binary')
        self.tx.recover()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')

    def test_sigkill_swapped_recover(self):
        self.crash('SWAPPED')
        self.tx.recover()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')

    def test_sigkill_verified_recover(self):
        self.crash('VERIFIED')
        self.tx.recover()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')

    def test_corrupt_backup_refuses_recovery_without_stop(self):
        self.crash('PREPARED')
        (self.tx.backup / 'openab-pty').write_bytes(b'corrupt')
        with self.assertRaises(B.Refusal):
            self.tx.recover()
        self.assertTrue(self.platform.active())
        self.assertTrue(self.tx.journal.exists())

    def test_corrupt_journal_refuses_recovery(self):
        self.crash('PREPARED')
        value = json.loads(self.tx.journal.read_text())
        value['backup'] = '/unknown'
        B.durable_json(self.tx.journal, value)
        with self.assertRaises(B.Refusal):
            self.tx.recover()
        self.assertTrue(self.platform.active())

    def test_ingress_concurrent_change_recovers_no_route_mutation(self):
        values = iter([self.platform.ingress(), self.platform.ingress(), '{"changed":true}', '{"changed":true}'])
        with patch.object(self.platform, 'ingress', side_effect=lambda: next(values)):
            with self.assertRaises(B.Refusal):
                self.tx.upgrade()
        self.assertEqual((self.root / 'openab-pty').read_bytes(), b'old-binary')
        self.assertTrue(self.platform.active())

    def test_lock_contention_owned_file(self):
        import fcntl
        with (self.parent / '.installer.lock').open('w') as first:
            fcntl.flock(first, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = subprocess.run(['flock', '-n', str(self.parent / '.installer.lock'), 'true'])
            self.assertNotEqual(result.returncode, 0)

    def test_loaded_state_pid_cgroup_and_unit_refusals(self):
        properties = {'LoadState': 'loaded', 'ActiveState': 'inactive', 'SubState': 'dead', 'UnitFileState': 'enabled',
                      'FragmentPath': str(self.unit), 'DropInPaths': '', 'Restart': 'on-failure', 'KillMode': 'control-group',
                      'MainPID': '0', 'ControlGroup': '', 'NeedDaemonReload': 'no'}
        platform = B.Platform(self.root, self.unit)
        def command(args, *unused, **kwargs):
            return ('# ' + str(self.unit) + '\n').encode() + self.unit.read_bytes() if 'cat' in args else \
                ''.join(k + '=' + v + '\n' for k, v in properties.items()).encode()
        with patch.object(B, 'run', side_effect=command):
            self.assertFalse(platform.snapshot(self.old)['active'])
            for key, bad in [('UnitFileState', 'masked'), ('ActiveState', 'failed'), ('LoadState', 'not-found'),
                             ('FragmentPath', '/unknown'), ('DropInPaths', '/override'), ('Restart', 'always'),
                             ('NeedDaemonReload', 'yes'), ('MainPID', '777'), ('ControlGroup', '/wrong')]:
                original = properties[key]
                properties[key] = bad
                with self.assertRaises(B.Refusal, msg=key):
                    platform.snapshot(self.old)
                properties[key] = original
            properties.update(ActiveState='active', SubState='running', MainPID=str(os.getpid()),
                              ControlGroup='/user.slice/user-' + str(os.getuid()) + '.slice/user@' + str(os.getuid()) + '.service/app.slice/' + B.UNIT)
            with patch.object(platform, 'members', return_value={os.getpid()}):
                with self.assertRaisesRegex(B.Refusal, 'executable path'):
                    platform.snapshot(self.old)
                with patch.object(B.os, 'readlink', return_value=str(self.root / 'openab-pty')):
                    with self.assertRaisesRegex(B.Refusal, 'digest mismatch'):
                        platform.snapshot(self.old)

    def test_invalid_bus_and_linger_refused(self):
        platform = B.Platform(self.root, self.unit)
        with patch.dict(os.environ, XDG_RUNTIME_DIR='/unknown'):
            with self.assertRaises(B.Refusal):
                platform.prerequisites()
        directory = SimpleNamespace(st_mode=stat.S_IFDIR | 0o700, st_uid=os.getuid())
        bus = SimpleNamespace(st_mode=stat.S_IFSOCK | 0o600, st_uid=os.getuid())
        with patch.dict(os.environ, XDG_RUNTIME_DIR='/run/user/' + str(os.getuid())), \
             patch.object(Path, 'lstat', autospec=True, side_effect=lambda p: bus if p.name == 'bus' else directory), \
             patch.object(B, 'run', side_effect=lambda args, *a: b'no\n' if 'loginctl' == args[0] else b'fixture\n'):
            with self.assertRaisesRegex(B.Refusal, 'Linger'):
                platform.prerequisites()
            bus.st_mode = stat.S_IFREG | 0o600
            with self.assertRaisesRegex(B.Refusal, 'bus'):
                platform.prerequisites()

    def test_nonempty_cgroup_after_stop_refused(self):
        platform = B.Platform(self.root, self.unit)
        with patch.object(B, 'run', return_value=b'0\n'), patch.object(platform, 'members', return_value={777}):
            with self.assertRaisesRegex(B.Refusal, 'cgroup not empty'):
                platform.stop({'cgroup': '/fixture-owned'})


if __name__ == '__main__':
    if sys.platform != 'linux' or os.getuid() == 0:
        raise SystemExit('Run on Linux/WSL as an ordinary user; no real systemd/Tailnet used.')
    unittest.main(verbosity=2)
