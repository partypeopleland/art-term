#!/usr/bin/env python3
"""Check the native IPC grants used by the development-mode setting."""
import json
from pathlib import Path
import tomllib
import unittest


GUI = Path(__file__).resolve().parents[1] / 'crates/art-term-gui'
COMMANDS = {'gui_runtime_development_mode', 'gui_runtime_set_development_mode'}


class DevelopmentModePermissions(unittest.TestCase):
    def test_settings_commands_are_allowed_on_each_platform(self):
        permissions = {
            permission['identifier']: permission
            for path in (GUI / 'permissions').glob('*.toml')
            for permission in tomllib.loads(path.read_text())['permission']
        }
        for filename in ('tauri.conf.json', 'tauri.android.conf.json'):
            with self.subTest(platform=filename):
                config = json.loads((GUI / filename).read_text())
                capabilities = config['app']['security']['capabilities']
                allowed = set()
                for capability in capabilities:
                    if isinstance(capability, str):
                        capability = json.loads(
                            (GUI / 'capabilities' / f'{capability}.json').read_text())
                    for grant in capability['permissions']:
                        if grant in permissions:
                            allowed.update(permissions[grant]['commands']['allow'])
                self.assertFalse(COMMANDS - allowed,
                                 f'Development settings IPC denied: {COMMANDS - allowed}')


if __name__ == '__main__':
    unittest.main()
