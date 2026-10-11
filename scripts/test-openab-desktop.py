#!/usr/bin/env python3
"""Isolated Linux desktop fixtures for the official format-4 bundle.

All writable fixtures stay under repo target/runtime-desktop-01. Requires a
filesystem preserving Linux owner/private modes; never weakens permission checks.
"""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

ASSETS = Path(__file__).resolve().parent / 'openab-runtime'
sys.path.insert(0, str(ASSETS))
import host_bundle as B
import desktop_bundle as D


class FakePlatform:
    def __init__(self, root):
        self.state = root / 'state'
        self.state.write_text('active')
        self.reloads = 0

    def prerequisites(self):
        pass

    def snapshot(self, manifest, allow_inactive=True):
        active = self.state.read_text() == 'active'
        B.require(allow_inactive or active, 'Fake inactive')
        return dict(active=active, enabled=True, pid=1 if active else 0, cgroup='/fixture')

    def stop(self, state):
        self.state.write_text('inactive')

    def start(self):
        self.state.write_text('active')

    def reload(self):
        self.reloads += 1

    def ready(self, official):
        B.require(self.snapshot({})['active'], 'Fake not ready')

    def ingress(self):
        return '{}'


def official_manifest(root, uid):
    """A consistent format-4 manifest for the fixture bundle contents."""
    value = {'format': 4, 'uid': uid, 'upstream_pin': B.PIN,
             'provenance': {'kind': 'provided-binary'}}
    for key, name in {**B.FILES, **B.LAUNCHER}.items():
        value[key] = B.digest((root / name).read_bytes())
    value['credential_verifier'] = B.digest((root / 'credential').read_bytes().strip())
    env = root / 'desktop.env'
    if env.exists():
        value['desktop_env'] = B.digest(env.read_bytes())
    return value


class DesktopTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=OUTPUT, prefix='fixture-')
        self.parent = Path(self.temp.name)
        self.root = self.parent / 'openab-pty'
        self.root.mkdir(mode=0o700)
        self.unit = self.parent / B.UNIT
        self.unit.write_bytes(('[Service]\nExecStart=' + str(self.root / 'openab-pty') + ' --config ' +
                               str(self.root / 'config.toml') + '\n').encode())
        self.unit.chmod(0o600)
        self.config = self.parent / 'desktop-config.json'
        B.durable_json(self.config, dict(mcp_url='http://192.0.2.10:8795/mcp', mcp_token='fixture-token'))
        raw = b'a' * 64
        files = dict(credential=raw + b'\n', **{'openab-pty': b'official-binary',
                'shell': b'#!/bin/bash\n', 'preview.py': b'#!/usr/bin/python3\n', 'mcp_preview.py': b'#!/usr/bin/python3\n',
                'config.toml': ('[pty]\nadmin_credential_hash = "sha256:' + B.digest(raw) + '"\n').encode(),
                'service': self.unit.read_bytes()})
        for name, data in files.items():
            path = self.root / name
            path.write_bytes(data)
            path.chmod(0o700 if name in ('openab-pty', 'shell', 'preview.py', 'mcp_preview.py') else 0o600)
        B.durable_json(self.root / 'manifest.json', official_manifest(self.root, os.getuid()))
        self.platform = FakePlatform(self.parent)
        self.tx = D.DesktopTransaction(B, self.root, self.unit, self.platform)

    def tearDown(self):
        self.temp.cleanup()

    def env_bytes(self):
        return D.environment(B, self.config)

    def test_enable_writes_env_and_manifest_without_service_touch(self):
        before = dict(self.platform.__dict__)
        self.tx.enable(self.config)
        m, data = B.verify(self.root, self.unit)
        self.assertEqual(m['format'], 4)
        self.assertEqual(data['desktop.env'], self.env_bytes())
        self.assertEqual(m['desktop_env'], B.digest(self.env_bytes()))
        self.assertEqual(self.platform.reloads, 0)
        self.assertEqual(self.platform.state.read_text(), 'active')

    def test_enable_noop_and_settings_change_refusal(self):
        self.tx.enable(self.config)
        self.tx.enable(self.config)
        B.durable_json(self.config, dict(mcp_url='http://192.0.2.10:8795/mcp', mcp_token='different-token'))
        with self.assertRaises(B.Refusal):
            self.tx.enable(self.config)

    def test_disable_keeps_empty_env_and_can_reenable(self):
        self.tx.enable(self.config)
        self.tx.rollback()
        m, data = B.verify(self.root, self.unit)
        self.assertEqual(m['desktop_env'], B.digest(b''))
        self.assertEqual(data['desktop.env'], b'')
        with self.assertRaises(B.Refusal):
            self.tx.rollback()
        self.tx.enable(self.config)
        self.assertEqual(B.verify(self.root, self.unit)[1]['desktop.env'], self.env_bytes())

    def test_format4_disable_ignores_retained_legacy_last(self):
        unit = self.unit.read_bytes() + ('EnvironmentFile=' + str(self.root / 'desktop.env') + '\n').encode()
        self.unit.write_bytes(unit)
        (self.root / 'service').write_bytes(unit)
        B.durable_json(self.root / 'manifest.json', official_manifest(self.root, os.getuid()))
        self.tx.enable(self.config)
        B.durable_json(self.tx.last, {'kind': 'desktop', 'phase': 'VERIFIED', 'legacy': True})
        self.tx.rollback()
        self.assertEqual(B.verify(self.root, self.unit)[1]['desktop.env'], b'')
        self.assertEqual(self.unit.read_bytes(), unit)
        self.assertEqual(self.platform.reloads, 0)
        self.assertTrue(self.tx.last.exists())

    def test_interrupted_stage_is_repaired_by_enable_and_disable(self):
        self.tx.enable(self.config)
        m = json.loads((self.root / 'manifest.json').read_text())
        del m['desktop_env']
        (self.root / 'manifest.json').write_text(json.dumps(m))
        with self.assertRaises(B.Refusal):
            B.verify(self.root, self.unit)
        self.tx.enable(self.config)
        B.verify(self.root, self.unit)
        # Atomic env replacement completed, but manifest still records the
        # old disabled file: retry must finish the existing two-file operation.
        m = json.loads((self.root / 'manifest.json').read_text())
        m['desktop_env'] = B.digest(b'')
        B.durable_json(self.root / 'manifest.json', m)
        self.tx.enable(self.config)
        B.verify(self.root, self.unit)
        (self.root / 'desktop.env').unlink()
        with self.assertRaises(B.Refusal):
            B.verify(self.root, self.unit)
        self.tx.rollback()
        B.verify(self.root, self.unit)

    def test_binary_arguments_and_pending_journals_refused(self):
        binary = self.parent / 'supplied-binary'
        binary.write_bytes(b'binary')
        with self.assertRaises(B.Refusal):
            self.tx.enable(self.config, str(binary), 'a' * 64)
        for name in ('.upgrade-intent.json', '.desktop-upgrade-intent.json'):
            path = self.parent / name
            path.write_text('{}')
            with self.assertRaises(B.Refusal):
                self.tx.enable(self.config)
            path.unlink()
        self.unit.write_bytes(b'unknown')
        with self.assertRaises(B.Refusal):
            self.tx.enable(self.config)

    def test_legacy_bundle_refused_until_upgrade(self):
        m = json.loads((self.root / 'manifest.json').read_text())
        m['format'] = 2
        (self.root / 'manifest.json').write_text(json.dumps(m))
        with self.assertRaisesRegex(B.Refusal, 'upgrade-runtime'):
            self.tx.enable(self.config)

    def test_configuration_fail_fast_before_side_effects(self):
        cases = [dict(mcp_url='http://192.0.2.10:8795', mcp_token='fixture-token'),
                 dict(mcp_url='http://192.0.2.10:8795/mcp/', mcp_token='fixture-token'),
                 dict(mcp_url='http://192.0.2.10:8795/nested/mcp', mcp_token='fixture-token'),
                 dict(mcp_url='http://' + 'a' * 2042 + '/mcp', mcp_token='fixture-token'),
                 dict(mcp_url='http://192.0.2.10:8795/mcp', mcp_token='a' * 4096 + '=')]
        for value in cases:
            B.durable_json(self.config, value)
            with self.assertRaises(B.Refusal):
                self.tx.enable(self.config)
            self.assertNotIn('desktop.env', [p.name for p in self.root.iterdir()])

    def test_private_config_and_token_validation(self):
        self.config.chmod(0o644)
        with self.assertRaises(B.Refusal):
            self.tx.enable(self.config)
        self.config.chmod(0o600)
        B.durable_json(self.config, dict(mcp_url='http://192.0.2.10/mcp', mcp_token='line\nbreak'))
        with self.assertRaises(B.Refusal):
            self.tx.enable(self.config)

    def legacy_bundle(self, root, fmt, unit_bytes):
        for path in root.iterdir():
            path.unlink()
        raw = b'a' * 64
        files = dict(credential=raw + b'\n', **{'openab-pty': b'patched-' + str(fmt).encode(),
                'shell': b'#!/bin/bash\n', 'service': unit_bytes,
                'config.toml': ('[pty]\nadmin_credential_hash = "sha256:' + B.digest(raw) + '"\n').encode()})
        if fmt == 3:
            files['mcp_preview.py'] = b'helper'
            files['desktop.env'] = b'env'
        for name, data in files.items():
            path = root / name
            path.write_bytes(data)
            path.chmod(0o700 if name in ('openab-pty', 'shell') else 0o600)
        m = {'format': fmt, 'uid': os.getuid(), 'upstream_pin': B.PIN,
             'provenance': {'kind': 'source-built'}, 'credential_verifier': B.digest(raw)}
        m.update({key: B.digest(files[name]) for key, name in B.FILES.items()})
        if fmt >= 2:
            m.update(compatibility_id=B.COMPAT, production_patch_sha256='b' * 64)
        if fmt == 3:
            m.update(desktop_compatibility='art-term-desktop-mcp-v1', desktop_patch_sha256='c' * 64,
                     helper_sha256=B.digest(files['mcp_preview.py']),
                     desktop_env_sha256=B.digest(files['desktop.env']))
        B.durable_json(root / 'manifest.json', m)
        return files, m

    def test_legacy_swapped_journal_recovers_format3_baseline(self):
        # A patch-era interrupted desktop transaction restores its baseline.
        backup = self.parent / '.desktop-upgrade-backup'
        backup.mkdir(mode=0o700)
        old_files, old_m = self.legacy_bundle(backup, 2, self.unit.read_bytes())
        root_files, root_m = self.legacy_bundle(self.root, 3, self.unit.read_bytes())
        v = dict(version=1, kind='desktop', uid=os.getuid(), root=str(self.root), unit=str(self.unit),
                 backup=str(backup), previous_binary=old_m['binary'], candidate_binary=root_m['binary'],
                 previous_manifest=B.digest(json.dumps(old_m, sort_keys=True, indent=2).encode() + b'\n'),
                 candidate_manifest=B.digest(json.dumps(root_m, sort_keys=True, indent=2).encode() + b'\n'),
                 previous_unit=old_m['unit'], candidate_unit=root_m['unit'],
                 old_enabled=True, old_active=True, ingress='{}', phase='SWAPPED')
        B.durable_json(self.tx.journal, v)
        self.tx.recover()
        m, data = B.verify(self.root, self.unit)
        self.assertEqual(m['format'], 2)
        self.assertEqual(data['openab-pty'], b'patched-2')


OUTPUT = Path(__file__).resolve().parent.parent / 'target/runtime-desktop-01'
if __name__ == '__main__':
    if sys.platform != 'linux' or os.getuid() == 0:
        raise SystemExit('Run on Linux/WSL as an ordinary user')
    OUTPUT.mkdir(mode=0o700, parents=True, exist_ok=True)
    OUTPUT.chmod(0o700)
    fd = B.directory(OUTPUT)
    os.close(fd)
    unittest.main(verbosity=2)
