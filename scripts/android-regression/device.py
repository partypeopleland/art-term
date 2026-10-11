"""ADB operations restricted to a root-created emulator ownership manifest."""
import json
import base64
import re
import subprocess
import time
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = 'net.missart.artterm'
STORE = 'no_backup/openab-operator-v1/connections.toml'
HIDDEN = getattr(subprocess, 'CREATE_NO_WINDOW', 0)


def require(value, code):
    if not value:
        raise RuntimeError(code)


def wait(predicate, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.15)
    raise RuntimeError('bounded wait expired')


class Device:
    def __init__(self, adb, serial, manifest):
        self.adb = Path(adb).resolve(strict=True)
        self.serial = serial
        require(re.fullmatch(r'emulator-\d+', serial), 'physical or unspecified device rejected')
        self.manifest = Path(manifest).resolve(strict=True)
        self.owner = json.loads(self.manifest.read_text(encoding='utf-8-sig'))
        require(self.owner['serial'] == serial, 'owned serial mismatch')
        self.reverse_ports, self.forward_ports = [], []
        self.verified = False
        self.verify()

    def call(self, *args, data=None, timeout=30):
        result = subprocess.run([str(self.adb), '-s', self.serial, *map(str, args)],
                                input=data, capture_output=True, timeout=timeout, creationflags=HIDDEN)
        require(result.returncode == 0, 'adb operation failed')
        return result.stdout

    def text(self, *args, **kwargs):
        return self.call(*args, **kwargs).decode('utf-8', errors='replace').strip()

    def verify(self):
        require(self.text('get-state') == 'device', 'owned emulator not online')
        name = self.text('emu', 'avd', 'name').splitlines()[0]
        require(name == self.owner['avdName'], 'owned AVD name mismatch')
        avd = Path(self.owner['avdPath']).resolve(strict=True)
        allowed = (ROOT / 'target/android-regression').resolve()
        require(avd.is_relative_to(allowed), 'owned AVD outside task root')
        require(Path(self.owner['root']).resolve().is_relative_to(allowed), 'owned root outside task root')
        require(Path(self.owner['root']).resolve() == self.manifest.parent and avd.parent == self.manifest.parent, 'ownership manifest path mismatch')
        require(Path(self.owner['adb']).resolve(strict=True) == self.adb, 'ownership ADB mismatch')
        for path in [avd, *avd.parents]:
            require(not path.is_symlink() and not path.is_junction(), 'owned AVD reparse rejected')
            if path == ROOT:
                break
        require(str(self.owner['pid']).isdigit() and self.owner['born'], 'missing process ownership identity')
        from environment import process_records
        known = {(int(p['pid']), str(p['born'])) for p in self.owner.get('processes', [])}
        require(known and any((int(p['pid']), str(p['born'])) in known for p in process_records(name)), 'owned emulator process identity changed')
        self.verified = True

    def install(self, apk, replace=False):
        self.verify()
        output = self.text('install', *(['-r'] if replace else []), str(apk), timeout=120)
        require('Success' in output, 'APK install not successful')

    def stop(self):
        self.call('shell', 'am', 'force-stop', PACKAGE)

    def start(self):
        self.call('shell', 'am', 'start', '-W', '-n', PACKAGE + '/.MainActivity')

    def clear(self):
        self.verify()
        require(self.text('shell', 'pm', 'clear', PACKAGE) == 'Success', 'owned package clear failed')

    def private_write(self, path, data):
        require(path in (STORE, 'no_backup/openab-operator-v1/openab-fixture-ca.der'), 'private write path rejected')
        self.call('shell', 'run-as', PACKAGE, 'mkdir', '-p', 'no_backup/openab-operator-v1')
        # One shell command preserves nested quoting; ASCII stdin preserves DER.
        command = "run-as " + PACKAGE + " sh -c 'base64 -d > " + path + "'"
        self.call('shell', '-T', command, data=base64.b64encode(data) + b'\n')
        require(self.call('exec-out','run-as',PACKAGE,'cat',path) == data, 'private write byte verification failed')

    def store(self):
        return self.call('exec-out', 'run-as', PACKAGE, 'cat', STORE)

    def reverse(self, port):
        self.call('reverse', f'tcp:{port}', f'tcp:{port}')
        self.reverse_ports.append(port)

    def forward(self, socket):
        port = int(self.text('forward', 'tcp:0', 'localabstract:' + socket))
        self.forward_ports.append(port)
        return port

    def key(self, code):
        self.call('shell', 'input', 'keyevent', str(code))

    def ui(self):
        self.call('shell', 'uiautomator', 'dump', '/sdcard/window.xml')
        return ET.fromstring(self.call('exec-out', 'cat', '/sdcard/window.xml'))

    def webview_bounds(self):
        # Accessibility exposes the native WebView and a nested document node
        # with the same class. Only a WebView without a WebView ancestor is the
        # actual native widget whose physical rectangle maps the CSS viewport.
        nodes = []
        def visit(node, in_webview=False):
            webview = node.get('package') == PACKAGE and node.get('class') == 'android.webkit.WebView'
            if webview and not in_webview:
                nodes.append(node)
            for child in node:
                visit(child, in_webview or webview)
        visit(self.ui())
        require(len(nodes) == 1, 'ambiguous native WebView bounds')
        bounds = list(map(int, re.findall(r'-?\d+', nodes[0].get('bounds', ''))))
        require(len(bounds) == 4 and bounds[2] > bounds[0] and bounds[3] > bounds[1], 'invalid native WebView bounds')
        return bounds

    def native_node(self, predicate, seconds=20):
        return wait(lambda: [n for n in self.ui().iter('node') if predicate(n)][:1], seconds)[0]

    def tap(self, node):
        bounds = list(map(int, re.findall(r'\d+', node.attrib['bounds'])))
        require(len(bounds) == 4, 'native bounds invalid')
        self.call('shell', 'input', 'tap', str((bounds[0]+bounds[2])//2), str((bounds[1]+bounds[3])//2))

    def documents(self):
        self.native_node(lambda n: n.get('package', '').endswith('documentsui'))

    def save_document(self, filename):
        require(re.fullmatch(r'[a-z0-9-]+\.json', filename), 'SAF filename rejected')
        self.documents()
        self.downloads()
        edit = self.native_node(lambda n: n.get('resource-id', '').endswith('title') and n.get('class') == 'android.widget.EditText')
        self.tap(edit)
        self.key('KEYCODE_MOVE_END')
        # Android input keycombination selects all without entering secrets.
        self.call('shell', 'input', 'keycombination', '113', '29')
        self.call('shell', 'input', 'text', filename)
        self.tap(self.native_node(lambda n: n.get('resource-id', '').endswith('button1') and n.get('enabled') == 'true'))

    def open_document(self, filename):
        self.documents()
        self.downloads()
        self.tap(self.native_node(lambda n: n.get('text') == filename))

    def downloads(self):
        # Open the drawer and choose Downloads, avoiding the Recents view.
        try:
            self.tap(self.native_node(lambda n: n.get('content-desc') in ('Show roots', '顯示根目錄'), 3))
            self.tap(self.native_node(lambda n: n.get('text') in ('Downloads', '下載') and n.get('resource-id') == 'android:id/title', 5))
        except RuntimeError:
            pass

    def cleanup(self):
        if not self.verified:
            return {}
        self.verify()
        self.stop()
        for port in self.forward_ports:
            self.call('forward', '--remove', f'tcp:{port}')
        for port in self.reverse_ports:
            self.call('reverse', '--remove', f'tcp:{port}')
        return {'appStopped': True, 'ownedForwardsRemoved': len(self.forward_ports), 'ownedReversesRemoved': len(self.reverse_ports), 'emulatorRetainedForRoot': True}
