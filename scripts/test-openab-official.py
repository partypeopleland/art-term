#!/usr/bin/env python3
"""Target-only official artifact and preview launcher checks (ordinary Linux user)."""
import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

ASSETS = Path(__file__).resolve().parent / 'openab-runtime'
sys.path.insert(0, str(ASSETS))
import official_artifact as A

OUTPUT = Path(__file__).resolve().parent.parent / 'target/official-runtime-host'
NONCE = '0123456789abcdef0123456789abcdef'
TOKEN = 'fixture-secret-must-not-appear'
PNG = b'\x89PNG\r\n\x1a\n' + b'fixture-image' * 100


class McpHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        if getattr(self.server, 'failure', False):
            self.send_response(500)
            self.end_headers()
            self.wfile.write(('private upstream failure ' + TOKEN).encode())
            return
        if self.headers.get('Authorization') != 'Bearer ' + TOKEN:
            self.send_error(401)
            return
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if request['method'] == 'notifications/initialized':
            self.send_response(202)
            self.end_headers()
            return
        if request['method'] == 'initialize':
            result = {'protocolVersion': '2025-03-26'}
        else:
            result = {'content': [{'type': 'image', 'mimeType': 'image/png',
                                   'data': base64.b64encode(getattr(self.server, 'image', PNG)).decode()}]}
        raw = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=OUTPUT, prefix='preview-fixture-')
        self.root = Path(self.temp.name)
        for name in ('preview.py', 'mcp_preview.py'):
            shutil.copyfile(ASSETS / name, self.root / name)
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), McpHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.config = self.root / 'desktop.env'
        self.config.write_text('ART_TERM_DESKTOP_MCP_URL=http://127.0.0.1:' +
                               str(self.server.server_port) + '/mcp\nHANDS_MCP_TOKEN=' + TOKEN + '\n')
        self.config.chmod(0o600)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.temp.cleanup()

    def launch(self, operation, fragmented=False):
        process = subprocess.Popen([sys.executable, str(self.root / 'preview.py'), operation, NONCE],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env={**os.environ, 'ART_TERM_PREVIEW_CONFIG': str(self.config)})
        if fragmented:
            # Deliberately consume arbitrary fragments, including frame markers
            # and JSON split across reads, as a PTY transport does.
            chunks = []
            while True:
                chunk = process.stdout.read(7)
                if not chunk:
                    break
                chunks.append(chunk)
            stdout = b''.join(chunks)
            stderr = process.stderr.read()
            process.wait(timeout=25)
            self.assertGreater(len(chunks), 20)
        else:
            stdout, stderr = process.communicate(timeout=25)
        self.assertNotIn(TOKEN.encode(), stdout + stderr)
        self.assertEqual(stderr, b'')
        self.assertTrue(stdout.startswith(b'\nARTTERM_PREVIEW_BEGIN:'))
        # bash emits this control prefix before a foreground command. The
        # launcher's newline must isolate the marker for the App's strict parser.
        prefixed = b'\x1b[?2004l\r' + stdout
        self.assertIn(b'\nARTTERM_PREVIEW_BEGIN:' + NONCE.encode() + b'\n', prefixed)
        lines = stdout.decode().splitlines()[1:]
        self.assertEqual(len(lines), 3)
        self.assertEqual(lines[0], 'ARTTERM_PREVIEW_BEGIN:' + NONCE)
        self.assertEqual(lines[2], 'ARTTERM_PREVIEW_END:' + NONCE + ':' + str(process.returncode))
        return process.returncode, json.loads(lines[1])

    def test_status_and_fragmented_screenshot_use_real_helper(self):
        code, value = self.launch('status')
        self.assertEqual((code, value), (0, {'enabled': True}))
        code, value = self.launch('screenshot', fragmented=True)
        self.assertEqual(code, 0)
        self.assertEqual(value['mime'], 'image/png')
        self.assertEqual(base64.b64decode(value['image_base64']), PNG)
        self.assertEqual(value['bytes'], len(PNG))

    def test_missing_disabled_and_insecure_config_fail_without_secret(self):
        for mode in ('insecure', 'disabled', 'missing'):
            if mode == 'insecure':
                self.config.chmod(0o644)
            elif mode == 'disabled':
                self.config.chmod(0o600)
                self.config.write_bytes(b'')
            else:
                self.config.unlink()
            code, value = self.launch('screenshot')
            self.assertEqual(code, 1)
            self.assertEqual(value, {'error': 'desktop preview unavailable'})

    def test_browser_sized_image_keeps_complete_framing_across_paced_chunks(self):
        self.server.image = PNG + b'large-preview-payload' * 30_000
        code, value = self.launch('screenshot', fragmented=True)
        self.assertEqual(code, 0)
        self.assertEqual(base64.b64decode(value['image_base64']), self.server.image)
        self.assertEqual(value['bytes'], len(self.server.image))

    def test_upstream_failure_body_is_not_echoed(self):
        self.server.failure = True
        code, value = self.launch('screenshot')
        self.assertEqual(code, 1)
        self.assertEqual(value, {'error': 'desktop MCP unavailable'})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--download', action='store_true', help='fetch pinned public GHCR artifact and verify every digest')
    args = parser.parse_args()
    OUTPUT.mkdir(parents=True, exist_ok=True)
    if args.download:
        binary = A.fetch()
        destination = OUTPUT / 'official-download-openab-pty'
        destination.write_bytes(binary)
        destination.chmod(0o700)
        print('OFFICIAL_ARTIFACT_PASS bytes=' + str(len(binary)) + ' sha256=' + hashlib.sha256(binary).hexdigest())
        return 0
    if sys.platform != 'linux' or os.getuid() == 0:
        parser.error('Launcher fixtures require ordinary Linux user/private filesystem modes')
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(LauncherTests))
    return 0 if result.wasSuccessful() else 1


if __name__ == '__main__':
    sys.exit(main())
