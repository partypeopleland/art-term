"""Verify the approval boundary advertised to real MCP clients over stdio."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'instance_bridge.py'


class ToolPermissions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with tempfile.TemporaryDirectory(prefix='art-term-permissions-') as directory:
            messages = [
                {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {}},
                {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list', 'params': {}},
            ]
            env = dict(os.environ, XDG_RUNTIME_DIR=directory)
            env.pop('ART_TERM_SCREEN_BINDING', None)
            result = subprocess.run(
                [sys.executable, str(SCRIPT), 'stdio', '--binding', 'permissions-test'],
                input=''.join(json.dumps(item) + '\n' for item in messages),
                text=True, capture_output=True, env=env, timeout=10, check=True,
            )
            response = next(json.loads(line) for line in result.stdout.splitlines()
                            if json.loads(line)['id'] == 2)
            cls.tools = {tool['name']: tool for tool in response['result']['tools']}

    def test_target_discovery_is_read_only(self):
        for name in ('current_target', 'list_target_tools'):
            with self.subTest(tool=name):
                self.assertIs(self.tools[name].get('annotations', {}).get('readOnlyHint'), True,
                              name + ' requires approval even though it only discovers the selected target')

    def test_desktop_dispatch_does_not_claim_to_be_read_only(self):
        self.assertIsNot(self.tools['call_target_tool'].get('annotations', {}).get('readOnlyHint'), True,
                         'the generic dispatcher can modify a desktop and must not claim to be read-only')


if __name__ == '__main__':
    unittest.main()
