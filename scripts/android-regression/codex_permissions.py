"""Run shared real Codex cases through an owned Android APK and native IPC."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('shared_codex_cases', ROOT / 'scripts/tui-regression/codex_permissions.py')
common = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = common
spec.loader.exec_module(common)

from device import PACKAGE, require, wait, runtime_work_name
from linux_emulator import LinuxEmulator
from ui_refresh import Refresh
from instance_direct import Screens


class AndroidSurface:
    def __init__(self, runtime, device, output):
        self.runtime, self.device, self.output = runtime, device, output
        self.suite = self.app = None
        self.stage = 'initialization'
        try:
            self.setup()
        except BaseException as error:
            diagnostic = {'stage': self.stage, 'error': str(error), 'action': getattr(self.app, 'action', None)}
            try:
                self.device.screenshot(output / 'setup-failure.png')
                if self.app:
                    raw = self.app.evaluate('document.documentElement.outerHTML')
                    for secret in [runtime.harness.credential, *(service.token for service in runtime.services)]:
                        raw = raw.replace(secret, '[redacted]')
                    (output / 'setup-failure.html').write_text(raw)
            except Exception as capture_error:
                diagnostic['captureError'] = type(capture_error).__name__
            (output / 'setup-failure.json').write_text(json.dumps(diagnostic, indent=2) + '\n')
            self.close()
            raise

    def setup(self):
        runtime, device = self.runtime, self.device
        device.clear()
        device.reverse(int(runtime.gateway.origin.rsplit(':', 1)[1]))
        for service in runtime.services:
            device.reverse(service.server.server_port)
        device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der', runtime.gateway.ca.read_bytes())
        self.suite = Refresh(device, runtime.gateway, self.output)
        self.stage = 'APK launch'
        self.app = self.suite.launch()
        self.stage = 'Host form'
        self.suite.settings()
        app = self.app
        app.set('name', 'Owned Codex Runtime')
        app.set('base-url', runtime.gateway.origin)
        app.set('credential', runtime.harness.credential)
        app.click_text('儲存', '[data-testid=runtime-host-form] button')
        wait(lambda: len(app.profiles()) == 1)
        self.suite.profile = app.profiles()[0]
        self.screens = Screens(self.suite, *runtime.services)
        for service in runtime.services:
            self.stage = 'Screen ' + service.name + ' form'
            self.screens.save(service)
        self.stage = 'Main page'
        self.suite.page('Main')
        wait(lambda: app.has('.new-connection') and not app.evaluate('document.querySelector(".new-connection").disabled'))
        app.click('.new-connection')
        self.stage = 'real work Shell binding'
        self.binding = runtime_work_name(app)
        self.label = app.text('.tab.active .name')
        self.stage = 'real work Shell prompt'
        wait(lambda: self.live() and 'ARTTERM_TUI_READY>' in self.text(), 45)
        require(any(item['name'] == self.binding for item in runtime.harness.api('GET', '/admin/sessions')[1]['sessions']),
                'APK binding was not created in the owned official Runtime')

    def live(self):
        return self.app.evaluate('!!document.querySelector(".tab.active .glyph.live")')

    def select(self, name):
        self.app.tap('desktop-toggle')
        wait(lambda: self.app.has('[data-testid=screen-picker]'))
        index = 0 if name == 'A' else 1
        self.app.tap('choose-screen-' + self.screens.ids[index])
        wait(lambda: not self.app.has('[data-testid=screen-picker]'), 45)
        require(self.app.evaluate('document.querySelector("[data-testid=desktop-toggle]").getAttribute("aria-pressed")==="true"'),
                'APK did not select the Screen')

    def clear(self):
        self.app.tap('desktop-toggle')
        self.app.tap('clear-screen')
        wait(lambda: not self.app.has('[data-testid=screen-picker]'))
        require(self.app.evaluate('document.querySelector("[data-testid=desktop-toggle]").getAttribute("aria-pressed")==="false"'),
                'APK did not clear the Screen')

    def submit(self, value):
        wait(self.live)
        self.app.tap('k-keyboard')
        require(self.app.evaluate('document.activeElement?.classList.contains("xterm-helper-textarea")===true'),
                'Keyboard action did not focus the real terminal input')
        self.app.command('Input.insertText', {'text': value})
        time.sleep(.3)
        self.app.tap('k-enter')

    def send(self, value):
        keys = {'\r': 'k-enter', '\x1b': 'k-esc'}
        require(value in keys, 'Only an observed approval action can use this key path')
        self.app.tap(keys[value])

    def pump(self):
        time.sleep(.08)

    def text(self):
        return self.app.evaluate('''(()=>{
          if(window.__scrollTest.nativeSnapshots.at(-1)?.visible)
            return window.__scrollTest.records.slice(-30).map(r=>r.text).join('\\n');
          const rows=[...document.querySelectorAll('.pane .xterm-rows > div')].filter(e=>e.getBoundingClientRect().width>0);
          return rows.length?rows.map(e=>e.textContent).join('\\n'):window.__scrollTest.records.slice(-30).map(r=>r.text).join('\\n');
        })()''')

    def save(self, output, name):
        self.device.screenshot(output / (name + '.png'))
        (output / (name + '.txt')).write_text(self.text())
        events = self.app.evaluate('({opens:window.__scrollTest.opens, nativeErrors:window.__scrollTest.nativeErrors, nativeSnapshots:window.__scrollTest.nativeSnapshots})')
        (output / (name + '-native.json')).write_text(json.dumps(events, indent=2) + '\n')

    def reopen(self):
        self.app.close()
        self.device.stop()
        self.app = self.suite.launch()
        wait(lambda: self.app.has('.existing-connections .attach-button'))
        if self.app.has('.existing-connections .connection-summary'):
            self.app.click('.existing-connections .connection-summary')
        selector = '.existing-connections .attach-button > span:first-child'
        wait(lambda: self.app.evaluate('[...document.querySelectorAll(' + json.dumps(selector) + ')].some(e=>e.textContent.trim()===' + json.dumps(self.label) + ')'))
        self.app.click_text(self.label, selector)
        require(runtime_work_name(self.app) == self.binding, 'App reopen attached another Shell')
        wait(self.live)

    def close(self):
        try:
            if self.app:
                self.app.close()
        finally:
            self.device.cleanup()
            self.device.forward_ports.clear()
            self.device.reverse_ports.clear()


def run_case(args, device):
    output = args.output_dir
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {'case': 'A-' + args.case, 'passed': False, 'platform': 'Android APK',
              'codexVersion': subprocess.check_output([str(args.codex), '--version'], text=True).strip(),
              'source': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
              'bridgeSha256': hashlib.sha256(args.bridge_source.read_bytes()).hexdigest(),
              'harnessSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'sharedHarnessSha256': hashlib.sha256(Path(common.__file__).read_bytes()).hexdigest(),
              'binarySha256': hashlib.sha256(args.binary.read_bytes()).hexdigest()}
    runtime = surface = codex = None
    try:
        with tempfile.TemporaryDirectory(prefix='art-term-') as directory:
            try:
                runtime = common.Runtime(args, Path(directory))
                surface = AndroidSurface(runtime, device, output)
                runtime.bindings.add(surface.binding)
                result['binding'] = surface.binding
                codex = common.Codex(runtime, surface, args.case, output)
                common.run_case(runtime, surface, codex, result)
                result['process'] = codex.identity()
                result['sessionId'] = codex.session_id()
                result['passed'] = True
            finally:
                try:
                    if codex:
                        codex.save()
                finally:
                    try:
                        if surface:
                            surface.close()
                    finally:
                        if runtime:
                            runtime.close()
    except Exception as error:
        result['passed'] = False
        result['error'] = str(error)
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    summary = dict(result)
    if summary.get('error'):
        summary['error'] = summary['error'].splitlines()[0]
    print(json.dumps(summary), flush=True)
    return result


def owned_device(args):
    sdk = os.environ.get('ANDROID_HOME')
    require(sdk, 'Set ANDROID_HOME to the installed Android SDK')
    run = ROOT / 'target/android-regression' / ('codex-' + uuid.uuid4().hex)
    device = LinuxEmulator(Path(sdk), run)
    try:
        device.start_emulator()
        device.install(args.binary)
        installed = device.text('shell', 'pm', 'path', PACKAGE).removeprefix('package:')
        require(device.text('shell', 'sha256sum', installed).split()[0] == hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                'Installed APK differs from the requested candidate')
        return device
    except BaseException:
        device.shutdown()
        raise


def close_device(device):
    try:
        device.cleanup()
    finally:
        device.shutdown()
    for name in ('device.avd', 'avd-index', 'android-user'):
        path = device.run / name
        if path.exists():
            shutil.rmtree(path)


def main():
    args = common.arguments()
    device = owned_device(args)
    try:
        result = run_case(args, device)
        return 0 if result['passed'] else 1
    finally:
        close_device(device)


if __name__ == '__main__':
    raise SystemExit(main())
