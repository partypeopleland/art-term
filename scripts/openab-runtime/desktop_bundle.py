#!/usr/bin/env python3
"""Optional desktop preview staging and bounded Host desktop transactions.

Public build inputs only. MCP secrets are read from an owner-only JSON file;
never pass its contents through argv, diagnostics or systemd manager environment.

On the official format-4 bundle, enabling the desktop only stages a protected
desktop.env the preview launcher reads at invocation time; the running service
is not restarted and Shells are not terminated. The legacy format-3 journal
recovery/rollback paths are retained for interrupted patch-era transactions.
"""
import argparse
import os
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit


def configuration(b, path):
    b.require(path is not None and path.is_absolute(), 'Desktop needs an absolute owner-only config file')
    # Parent may be a conventional ~/.config directory; file must still be 0600.
    value = b.json_bytes(b.private_read(path, False))
    b.require(isinstance(value, dict) and set(value) == {'mcp_url', 'mcp_token'}, 'Invalid desktop config fields')
    url, token = value['mcp_url'], value['mcp_token']
    b.require(isinstance(url, str) and isinstance(token, str), 'Invalid desktop config types')
    parsed = urlsplit(url)
    b.require(parsed.scheme in ('http', 'https') and parsed.hostname and not parsed.username and
              not parsed.password and not parsed.fragment and not parsed.query and
              parsed.path == '/mcp' and len(url) <= 2048 and
              re.fullmatch(r'[A-Za-z0-9:/._\[\]-]+', url), 'Invalid desktop MCP URL')
    # Restrict to safe bearer characters and exclude expansion/newlines.
    b.require(re.fullmatch(r'[A-Za-z0-9._~+/-]{1,4096}=*', token) and len(token) <= 4096,
              'Invalid desktop MCP token encoding')
    return url, token


def environment(b, config):
    url, token = configuration(b, config)
    return ('ART_TERM_DESKTOP_MCP_URL=' + url + '\nART_TERM_DESKTOP_MCP_TOKEN_ENV=HANDS_MCP_TOKEN\n' +
            'HANDS_MCP_TOKEN=' + token + '\n').encode()


def stage(b, destination, config):
    """Fresh-install staging: the launchers are already part of the bundle."""
    path = destination / 'desktop.env'
    path.write_bytes(environment(b, config))
    path.chmod(0o600)


def write_private(b, directory_path, name, raw):
    # Same-directory atomic replacement, without publishing a partial file.
    fd = b.directory(directory_path)
    temporary = '.desktop-env-' + os.urandom(12).hex()
    child = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
    try:
        with os.fdopen(child, 'wb', closefd=False) as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(child)
        os.rename(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
        os.fsync(fd)
    finally:
        os.close(child)
        try:
            os.unlink(temporary, dir_fd=fd)
        except FileNotFoundError:
            pass
        os.close(fd)


def remove_private(b, directory_path, name):
    fd = b.directory(directory_path)
    try:
        try:
            os.unlink(name, dir_fd=fd)
        except FileNotFoundError:
            pass
        os.fsync(fd)
    finally:
        os.close(fd)


def write_unit(b, unit, raw):
    # Same-directory atomic replacement, without publishing a partially written unit.
    fd = b.directory(unit.parent, False)
    temporary = '.desktop-unit-' + os.urandom(12).hex()
    child = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
    try:
        with os.fdopen(child, 'wb', closefd=False) as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(child)
        os.rename(temporary, unit.name, src_dir_fd=fd, dst_dir_fd=fd)
        os.fsync(fd)
    finally:
        os.close(child)
        try:
            os.unlink(temporary, dir_fd=fd)
        except FileNotFoundError:
            pass
        os.close(fd)


def loose(b, root, unit):
    """Format-4 manifest/data tolerant only about desktop.env consistency.

    A crash between staging desktop.env and recording its digest must remain
    repairable by re-running enable/rollback; everything else stays strict.
    """
    fd = b.directory(root)
    try:
        names = set(os.listdir(fd))
        b.require('manifest.json' in names, 'Missing managed manifest')
        data = {name: b.read_at(fd, name) for name in names}
    finally:
        os.close(fd)
    m = b.json_bytes(data['manifest.json'])
    b.require(isinstance(m, dict) and m.get('format') == b.OFFICIAL_FORMAT and
              type(m.get('uid')) is int and m.get('uid') == os.getuid(),
              'Desktop operations need the official format-4 bundle; migrate with --upgrade-runtime')
    expected = {'openab-pty', 'config.toml', 'shell', 'service', 'credential', 'manifest.json',
                'preview.py', 'mcp_preview.py'}
    b.require(expected <= names, 'Missing bundle artifact; inspect before changing desktop')
    extra = names - expected
    b.require(extra <= {'desktop.env'}, 'Unexpected bundle artifact; inspect before changing desktop')
    for key, name in {**b.FILES, **b.LAUNCHER}.items():
        b.require(isinstance(m.get(key), str) and b.HEX.fullmatch(m[key]) and
                  b.digest(data[name]) == m[key], 'Managed digest mismatch: ' + name)
    b.require(data['service'] == b.private_read(unit, False), 'Managed unit bytes mismatch')
    credential = data['credential'].strip()
    b.require(re.fullmatch(b'[a-f0-9]{64}', credential) and
              b.digest(credential) == m.get('credential_verifier', ''), 'Credential verifier mismatch')
    matches = re.findall(rb'^admin_credential_hash\s*=\s*"(sha256:[a-f0-9]{64})"\s*$',
                         data['config.toml'], re.M)
    b.require(matches == [('sha256:' + m['credential_verifier']).encode()], 'Config verifier mismatch')
    if 'desktop.env' in names and 'desktop_env' in m:
        b.require(isinstance(m['desktop_env'], str) and b.HEX.fullmatch(m['desktop_env']),
                  'Invalid desktop config digest')
    return m, data


class DesktopTransaction:
    def __init__(self, b, root, unit, platform=None):
        self.b, self.root, self.unit = b, root, unit
        self.parent = root.parent
        self.backup = self.parent / '.desktop-upgrade-backup'
        self.journal = self.parent / '.desktop-upgrade-intent.json'
        self.last = self.parent / '.desktop-upgrade-last.json'
        self.platform = platform or b.Platform(root, unit)

    def reload(self):
        self.platform.reload()

    def phase(self, value, phase):
        value['phase'] = phase
        self.b.durable_json(self.journal, value)

    def pairs(self, v):
        b = self.b
        a, ad = b.verify(self.root, None)
        c, cd = b.verify(self.backup, None)
        ah, ch = b.digest(ad['manifest.json']), b.digest(cd['manifest.json'])
        b.require((ah, ch) in [(v['previous_manifest'], v['candidate_manifest']),
                              (v['candidate_manifest'], v['previous_manifest'])], 'Unknown desktop bundle pair')
        old, new = (ad, cd) if ah == v['previous_manifest'] else (cd, ad)
        b.require(b.digest(old['service']) == v['previous_unit'] and b.digest(new['service']) == v['candidate_unit'] and
                  b.digest(old['openab-pty']) == v['previous_binary'] and
                  b.digest(new['openab-pty']) == v['candidate_binary'], 'Desktop journal artifact mismatch')
        for name in ('credential', 'config.toml', 'shell'):
            b.require(old[name] == new[name], 'Desktop enrollment continuity mismatch: ' + name)
        b.require(b.private_read(self.unit, False) in (old['service'], new['service']), 'Unknown external unit modification')
        return a, ad, ah

    def read_journal(self, path):
        b = self.b
        b.require(not (self.parent / '.upgrade-intent.json').exists() and
                  not (self.parent / '.upgrade-intent.json').is_symlink(), 'Legacy upgrade pending; desktop recovery refused')
        v = b.json_bytes(b.private_read(path))
        b.require(isinstance(v, dict) and type(v.get('version')) is int and v['version'] == 1 and
                  type(v.get('uid')) is int and v['uid'] == os.getuid() and v.get('kind') == 'desktop' and
                  v.get('root') == str(self.root) and v.get('unit') == str(self.unit) and
                  v.get('backup') == str(self.backup) and v.get('phase') in
                  ('PREPARED', 'STOPPED', 'SWAPPED', 'VERIFIED', 'RECOVERING') and
                  v.get('old_enabled') is True and type(v.get('old_active')) is bool, 'Invalid desktop journal')
        for key in ('previous_manifest', 'candidate_manifest', 'previous_binary', 'candidate_binary', 'previous_unit', 'candidate_unit'):
            b.require(isinstance(v.get(key), str) and b.HEX.fullmatch(v[key]), 'Invalid desktop journal identity')
        b.require(isinstance(v.get('ingress'), str) and isinstance(b.json_bytes(v['ingress']), dict), 'Invalid desktop ingress record')
        return v

    def recover(self):
        b = self.b
        self.platform.prerequisites()
        v = self.read_journal(self.journal)
        m, data, root_hash = self.pairs(v)
        # Either known unit may be on disk after a crash. Reconcile it to the
        # currently installed bundle before using normal loaded-unit checks.
        write_unit(b, self.unit, data['service'])
        self.reload()
        state = self.platform.snapshot(m)
        self.phase(v, 'RECOVERING')
        if state['active']:
            self.platform.stop(state)
        self.pairs(v)
        if root_hash == v['candidate_manifest']:
            b.exchange(self.parent, self.root.name, self.backup.name)
        old, data = b.verify(self.root, None)
        b.require(b.digest(data['manifest.json']) == v['previous_manifest'], 'Original desktop baseline not restored')
        write_unit(b, self.unit, data['service'])
        self.reload()
        b.verify(self.root, self.unit)
        if v['old_active']:
            self.platform.start()
            self.platform.ready(old['format'] == b.OFFICIAL_FORMAT)
            self.platform.snapshot(old, False)
        else:
            b.require(not self.platform.snapshot(old)['active'], 'Original inactive state not restored')
        preserved = self.platform.ingress() == v['ingress']
        v.update(phase='RECOVERED', ingress_preserved=preserved)
        b.durable_json(self.last, v)
        self.journal.unlink()
        b.sync_dir(self.parent)
        b.require(preserved, 'Original bundle restored; ingress changed/unproven, inspect manually')
        print('Desktop baseline and external unit restored; terminated Shells cannot be restored. Backup retained.')

    def rollback(self):
        b = self.b
        current = b.json_bytes(b.private_read(self.root / 'manifest.json'))
        if current.get('format') == b.OFFICIAL_FORMAT:
            self.disable()
            return
        b.Transaction(self.root, self.unit).pending()
        v = self.read_journal(self.last)
        b.require(v['phase'] == 'VERIFIED', 'No successful desktop activation available for rollback')
        _, _, current = self.pairs(v)
        b.require(current == v['candidate_manifest'], 'Desktop rollback current identity mismatch')
        b.verify(self.root, self.unit)
        b.durable_json(self.journal, v)
        self.recover()

    def enable(self, config, supplied='', expected=''):
        b = self.b
        b.Transaction(self.root, self.unit).pending()
        b.require(not supplied and not expected,
                  'A replacement binary cannot be applied during desktop activation')
        env = environment(b, config)
        m, data = loose(b, self.root, self.unit)
        if data.get('desktop.env') and m.get('desktop_env') == b.digest(data['desktop.env']):
            b.require(data['desktop.env'] == env, 'Desktop settings differ; use --rollback-desktop then re-enable')
            print('Desktop preview already enabled with identical settings; no changes.')
            return
        # Otherwise (re)write both halves consistently; this also repairs a
        # crash between staging the file and recording its manifest digest.
        write_private(b, self.root, 'desktop.env', env)
        m['desktop_env'] = b.digest(env)
        b.durable_json(self.root / 'manifest.json', m)
        b.sync_dir(self.root)
        b.verify(self.root, self.unit)
        print('Desktop preview enabled; the launcher reads the protected config per invocation. '
              'No service restart and no Shell was terminated.')

    def disable(self):
        b = self.b
        b.Transaction(self.root, self.unit).pending()
        b.require(not (self.parent / '.desktop-upgrade-intent.json').exists() and
                  not (self.parent / '.desktop-upgrade-intent.json').is_symlink(),
                  'Interrupted desktop transaction pending; use --recover-desktop')
        m, data = loose(b, self.root, self.unit)
        b.require('desktop_env' in m or 'desktop.env' in data, 'Desktop preview is not enabled')
        b.require(data.get('desktop.env') != b'', 'Desktop preview is not enabled')
        # A migrated format-3 unit still requires this EnvironmentFile. Keep
        # an empty protected file so its next start succeeds; the launcher
        # treats it as disabled. No unit rewrite or restart is needed.
        write_private(b, self.root, 'desktop.env', b'')
        m['desktop_env'] = b.digest(b'')
        b.durable_json(self.root / 'manifest.json', m)
        b.sync_dir(self.root)
        b.verify(self.root, self.unit)
        print('Desktop preview disabled; the launcher now reports unavailable. No service restart needed.')


def main():
    import host_bundle as b
    parser = argparse.ArgumentParser(description='Assemble the pinned official runtime and launchers; no deployment')
    parser.add_argument('--destination', type=Path, required=True)
    args = parser.parse_args()
    try:
        b.require(args.destination.is_absolute(), 'Build destination must be absolute')
        b.ancestry(args.destination)
        args.destination.mkdir(mode=0o700)
        b.build(args.destination)
        print('Official runtime and preview launchers assembled; digests in build-provenance.json.')
    except (b.Refusal, OSError, ValueError) as error:
        print('ERROR: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
