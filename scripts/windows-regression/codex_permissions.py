"""Drive real Windows WebView2 through SSH; keep actual Codex auth on its Host."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import uuid

import requests
from selenium import webdriver
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.proxy import Proxy, ProxyType
from selenium.webdriver.remote.client_config import ClientConfig

import webview


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('windows_codex_cases', ROOT / 'scripts/tui-regression/codex_permissions.py')
common = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = common
spec.loader.exec_module(common)


def ps(value):
    return "'" + str(value).replace("'", "''") + "'"


def encoded(source):
    return base64.b64encode(source.encode('utf-16le')).decode()


class Windows:
    def __init__(self, args, runtime):
        self.args, self.runtime = args, runtime
        self.root = args.windows_root.rstrip('\\/') + '\\' + uuid.uuid4().hex
        self.task = 'art-term-issue8-' + uuid.uuid4().hex
        self.tunnel = None
        self.processes = []
        self.started = False
        self.prepared = False

    def run(self, source):
        command = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); $ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; " + source
        reply = subprocess.run(['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10',
                                self.args.windows_host, 'pwsh.exe -NoProfile -NonInteractive -EncodedCommand ' + encoded(command)],
                               capture_output=True, timeout=40)
        if reply.returncode:
            raise RuntimeError('Windows operation failed: ' + reply.stderr.decode(errors='replace')[-1500:])
        return json.loads(reply.stdout.decode('utf-8-sig').strip() or 'null')

    def prepare(self):
        metadata = self.run("if(Get-Process art-term-gui -ErrorAction SilentlyContinue){throw 'Existing App must be preserved'}; "
                            "$r=" + ps(self.root) + "; if(-not [IO.Path]::GetFullPath($r).StartsWith(([IO.Path]::GetTempPath()+'art-term-'),[StringComparison]::OrdinalIgnoreCase)){throw 'Tests require an art-term temporary directory'}; "
                            "New-Item -ItemType Directory $r -ErrorAction Stop | Out-Null; "
                            "$p=New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,0); $p.Start(); $port=$p.LocalEndpoint.Port; $p.Stop(); "
                            "@{port=$port;sha=(Get-FileHash " + ps(self.args.windows_exe) + " -Algorithm SHA256).Hash.ToLower()} | ConvertTo-Json -Compress")
        self.remote_port = metadata['port']
        self.sha = metadata['sha']
        self.prepared = True
        cert = base64.b64encode(self.runtime.gateway.ca.read_bytes()).decode()
        self.run('[IO.File]::WriteAllBytes(' + ps(self.root + '\\ca.der') + ',[Convert]::FromBase64String(' + ps(cert) + '))')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            local_port = sock.getsockname()[1]
        ports = [int(self.runtime.gateway.origin.rsplit(':', 1)[1]),
                 *(service.server.server_port for service in self.runtime.services)]
        command = ['ssh', '-N', '-o', 'BatchMode=yes', '-o', 'ExitOnForwardFailure=yes',
                   '-o', 'ServerAliveInterval=10', '-L', f'{local_port}:127.0.0.1:{self.remote_port}']
        for port in ports:
            command += ['-R', f'127.0.0.1:{port}:127.0.0.1:{port}']
        self.log = (self.args.output_dir / 'tunnel.log').open('ab')
        self.tunnel = subprocess.Popen([*command, self.args.windows_host], stdout=self.log, stderr=self.log)
        self.url = f'http://127.0.0.1:{local_port}'

    def start(self):
        command = ("$ErrorActionPreference='Stop'; $env:WEBVIEW2_USER_DATA_FOLDER=" + ps(self.root + '\\webview') + '; '
                   '$env:ART_TERM_OPENAB_FIXTURE_CA=' + ps(self.root + '\\ca.der') + '; '
                   'Remove-Item Env:ART_TERM_OPENAB_FIXTURE_DATA,Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue; '
                   '& ' + ps(self.args.edge_driver) + ' ' + ps('--port=' + str(self.remote_port)) +
                   ' *> ' + ps(self.root + '\\driver.log'))
        self.driver_command = encoded(command)
        self.started = True
        self.run('$a=New-ScheduledTaskAction -Execute (Get-Command pwsh.exe).Source -Argument ' +
                 ps('-NoProfile -NonInteractive -EncodedCommand ' + self.driver_command) +
                 '; $p=New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited; '
                 '$s=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 15); '
                 'Register-ScheduledTask -TaskName ' + ps(self.task) + ' -Action $a -Principal $p -Settings $s | Out-Null; '
                 'Start-ScheduledTask -TaskName ' + ps(self.task))
        http = requests.Session()
        http.trust_env = False
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            if self.tunnel.poll() is not None:
                raise RuntimeError('Owned SSH tunnel exited')
            try:
                if http.get(self.url + '/status', timeout=1).status_code == 200:
                    return
            except requests.RequestException:
                pass
            time.sleep(.2)
        raise RuntimeError('Owned interactive EdgeDriver did not start')

    def capture_processes(self):
        current = self.run("$all=@(Get-CimInstance Win32_Process); $roots=@($all | Where-Object {$_.Name -eq 'msedgedriver.exe' -and $_.CommandLine -match " +
                                  ps('--port=' + str(self.remote_port) + '(?:\\s|$)') + "}); if($roots.Count -eq 0){'[]'; return}; if($roots.Count -ne 1){throw 'Ambiguous driver ownership'}; "
                                  '$parent=@($all | Where-Object {$_.ProcessId -eq $roots[0].ParentProcessId}); '
                                  'if($roots[0].ExecutablePath -ne ' + ps(self.args.edge_driver) +
                                  ' -or $parent.Count -ne 1 -or -not $parent[0].CommandLine.Contains(' + ps(self.driver_command) +
                                  ')){throw "Driver is not owned by this scheduled action"}; '
                                  '$ids=@{}; $ids[[uint32]$roots[0].ProcessId]=$roots[0].CreationDate; do {$next=@($all | Where-Object {$ids.ContainsKey([uint32]$_.ParentProcessId) -and -not $ids.ContainsKey([uint32]$_.ProcessId) -and $_.CreationDate -ge $ids[[uint32]$_.ParentProcessId]}); foreach($c in $next){$ids[[uint32]$c.ProcessId]=$c.CreationDate}}while($next.Count); '
                                  'ConvertTo-Json -Compress -InputObject @($all | Where-Object {$ids.ContainsKey([uint32]$_.ProcessId)} | ForEach-Object {@{pid=$_.ProcessId;born=$_.CreationDate.ToUniversalTime().Ticks.ToString();name=$_.Name}})')
        known = {(p['pid'], p['born']) for p in self.processes}
        self.processes.extend(p for p in current if (p['pid'], p['born']) not in known)
        return current

    def stop_processes(self, include_driver):
        owned = [p for p in self.processes if include_driver or p['name'] != 'msedgedriver.exe']
        if owned:
            self.run('$owned=ConvertFrom-Json ' + ps(json.dumps(owned)) + '; '
                     'foreach($item in $owned){$p=Get-CimInstance Win32_Process -Filter ("ProcessId="+$item.pid); '
                     'if($p -and $p.CreationDate.ToUniversalTime().Ticks.ToString() -eq $item.born){Stop-Process -Id $item.pid -Force -ErrorAction SilentlyContinue}}; '
                     '$all=@(Get-CimInstance Win32_Process); foreach($item in $owned){'
                     'if($all | Where-Object {$_.ProcessId -eq $item.pid -and $_.CreationDate.ToUniversalTime().Ticks.ToString() -eq $item.born}){throw "Owned process did not stop"}}')

    def close(self):
        try:
            if self.started:
                try:
                    self.capture_processes()
                    self.stop_processes(include_driver=True)
                finally:
                    self.run('$t=Get-ScheduledTask -TaskName ' + ps(self.task) + ' -ErrorAction SilentlyContinue; '
                             'if($t){if($t.Actions.Arguments -ne ' + ps('-NoProfile -NonInteractive -EncodedCommand ' + self.driver_command) +
                             '){throw "Scheduled action ownership changed"}; Stop-ScheduledTask -TaskName ' + ps(self.task) +
                             ' -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName ' + ps(self.task) + ' -Confirm:$false}')
                self.started = False
            if self.prepared:
                self.run('if(Get-CimInstance Win32_Process | Where-Object {$_.CommandLine -and $_.CommandLine.Replace("/","\\").Contains(' +
                         ps(self.root) + ')}){throw "Owned run directory is still in use"}; '
                         'Remove-Item -LiteralPath ' + ps(self.root) + ' -Recurse -Force')
                self.prepared = False
                (self.args.output_dir / 'windows-cleanup.json').write_text(json.dumps(
                    {'root': self.root, 'task': self.task, 'ownedProcesses': self.processes, 'removed': True}, indent=2) + '\n')
        finally:
            if self.tunnel:
                self.tunnel.terminate()
                self.tunnel.wait(timeout=10)
                self.tunnel = None
                self.log.close()


class App(webview.App):
    def __init__(self, windows):
        self.windows = windows
        self.driver = None
        self.metadata = {}

    def launch(self):
        options = webdriver.EdgeOptions()
        options.use_webview = True
        options.binary_location = self.windows.args.windows_exe
        options.add_argument('--config=' + self.windows.root + '\\connections.toml')
        client = ClientConfig(remote_server_addr=self.windows.url, timeout=55,
                              proxy=Proxy({'proxyType': ProxyType.DIRECT['string']}))
        self.driver = webdriver.Remote(self.windows.url, options=options, client_config=client)
        self.driver.set_script_timeout(45)
        self.driver.command_executor._commands['executeCdpCommand'] = ('POST', '/session/$sessionId/ms/cdp/execute')
        self.wait(lambda: self.find('[data-testid=desktop-workspace-layout]'))
        self.metadata = {'observer': self.driver.execute_script(webview.OBSERVER),
                         'capabilities': self.driver.capabilities}
        version = self.driver.capabilities['browserVersion']
        driver_version = self.driver.capabilities.get('msedge', {}).get('msedgedriverVersion', '')
        webview.require(driver_version.startswith('.'.join(version.split('.')[:3]) + '.'), 'WebView2/driver mismatch')
        self.windows.capture_processes()
        return self

    def close(self):
        if self.driver:
            self.windows.capture_processes()
            try:
                self.driver.quit()
            finally:
                self.driver = None
                self.windows.stop_processes(include_driver=False)


class WindowsSurface:
    def __init__(self, runtime):
        self.runtime = runtime
        self.windows = Windows(runtime.args, runtime)
        self.app = None
        try:
            self.windows.prepare()
            self.windows.start()
            self.app = App(self.windows).launch()
            self.setup()
        except BaseException as error:
            diagnostic = {'error': str(error), 'windowsRoot': self.windows.root}
            if self.app and self.app.driver:
                try:
                    self.save(runtime.args.output_dir, 'setup-failure')
                    raw = self.app.driver.execute_script('return document.documentElement.outerHTML')
                    for secret in [runtime.harness.credential, *(s.token for s in runtime.services)]:
                        raw = raw.replace(secret, '[redacted]')
                    (runtime.args.output_dir / 'setup-failure.html').write_text(raw)
                except Exception as capture_error:
                    diagnostic['captureError'] = type(capture_error).__name__
            (runtime.args.output_dir / 'setup-failure.json').write_text(json.dumps(diagnostic, indent=2) + '\n')
            self.close()
            raise

    def setup(self):
        app, runtime = self.app, self.runtime
        app.manage()
        app.fill('name', 'Owned Codex Runtime')
        app.fill('base-url', runtime.gateway.origin)
        app.fill('credential', runtime.harness.credential)
        app.click('儲存', '[data-testid=runtime-host-form] button')
        app.wait(lambda: len(app.profiles()) == 1)
        app.settings()
        self.ids = []
        for service in runtime.services:
            app.fill('screen-name', 'Screen ' + service.name)
            app.fill('screen-url', service.url)
            app.fill('screen-token', service.token)
            app.click('儲存 Screen', '[data-testid=screen-form] button')
            app.wait(lambda: len(app.screens()) == len(self.ids) + 1)
            app.wait(lambda: app.node('screen-name').is_enabled())
            self.ids.append(next(s['id'] for s in app.screens() if s['name'] == 'Screen ' + service.name))
        app.main_page()
        app.wait(lambda: next((e for e in app.find('.new-connection') if e.is_enabled() and e.is_displayed()), None)).click()
        app.wait(lambda: self.live() and 'ARTTERM_TUI_READY>' in self.text(), 45)
        self.binding = next(s['name'].removeprefix('openab:') for s in app.driver.execute_script('return Object.values(window.__reg.sessions)') if s['name'].startswith('openab:th2-'))
        self.label = app.find('.tab.active .name')[0].text
        webview.require(any(s['name'] == self.binding for s in runtime.harness.api('GET', '/admin/sessions')[1]['sessions']), 'Windows did not create an owned official work Shell')

    def live(self):
        return self.app.driver.execute_script('return !!document.querySelector(".tab.active .glyph.live")')

    def select(self, name):
        app = self.app
        app.node('desktop-toggle').click()
        app.node('choose-screen-' + self.ids[0 if name == 'A' else 1]).click()
        app.wait(lambda: not app.find('[data-testid=screen-picker]'))
        app.wait(lambda: app.node('desktop-toggle').get_attribute('aria-pressed') == 'true' and app.node('desktop-toggle').is_enabled(), 45)

    def clear(self):
        self.app.node('desktop-toggle').click()
        self.app.node('clear-screen').click()
        self.app.wait(lambda: not self.app.find('[data-testid=screen-picker]'))
        self.app.wait(lambda: self.app.node('desktop-toggle').get_attribute('aria-pressed') == 'false' and self.app.node('desktop-toggle').is_enabled(), 45)

    def submit(self, value):
        self.app.wait(self.live)
        textarea = self.app.wait(lambda: self.app.driver.execute_script('return [...document.querySelectorAll(".pane .xterm-helper-textarea")].find(e=>e.closest(".pane").getBoundingClientRect().width>0) || null'))
        self.app.driver.execute_script('arguments[0].focus()', textarea)
        self.app.driver.execute('executeCdpCommand', {'cmd': 'Input.insertText', 'params': {'text': value}})
        time.sleep(.3)
        ActionChains(self.app.driver).send_keys(Keys.RETURN).perform()

    def send(self, value):
        webview.require(value in ('\r', '\x1b'), 'Only observed approval keys are allowed')
        ActionChains(self.app.driver).send_keys(Keys.RETURN if value == '\r' else Keys.ESCAPE).perform()

    def pump(self):
        time.sleep(.08)

    def text(self):
        return self.app.driver.execute_script('return [...document.querySelectorAll(".pane .xterm-rows > div")].filter(e=>e.getBoundingClientRect().width>0).map(e=>e.textContent).join("\\n")')

    def save(self, output, name):
        self.app.driver.save_screenshot(str(output / (name + '.png')))
        (output / (name + '.txt')).write_text(self.text())
        (output / (name + '-native.json')).write_text(json.dumps(self.app.metadata, indent=2) + '\n')

    def reopen(self):
        self.app.close()
        self.app = App(self.windows).launch()
        self.app.wait(lambda: self.app.find('.existing-connections .attach-button'))
        summaries = self.app.find('.existing-connections .connection-summary')
        if summaries and not any(e.is_displayed() for e in self.app.find('.existing-connections .attach-button:not(.connection-summary)')):
            summaries[0].click()
        self.app.click(self.label, '.existing-connections .attach-button > span:first-child')
        self.app.wait(self.live)
        names = self.app.driver.execute_script('return Object.values(window.__reg.sessions).map(s=>s.name)')
        webview.require('openab:' + self.binding in names, 'Windows reopened another Shell')

    def close(self):
        try:
            if self.app:
                self.app.close()
        finally:
            self.windows.close()


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('runtime-binary', 'history-bundle', 'node', 'tmux', 'codex', 'auth-file', 'output-dir'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('windows-host', 'windows-root', 'windows-exe', 'edge-driver'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--bridge-source', type=Path, default=ROOT / 'scripts/instance_bridge.py')
    parser.add_argument('--model', default='gpt-6.1-sol')
    parser.add_argument('--case', choices=['C%02d' % n for n in range(1, 15)], required=True)
    parser.add_argument('--timeout', type=int, default=150)
    args = parser.parse_args()
    for name in ('runtime_binary', 'history_bundle', 'node', 'tmux', 'codex', 'auth_file', 'bridge_source'):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    args.output_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {'case': 'W-' + args.case, 'platform': 'Windows EXE/WebView2', 'passed': False,
              'source': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
              'bridgeSha256': hashlib.sha256(args.bridge_source.read_bytes()).hexdigest(),
              'harnessSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'sharedHarnessSha256': hashlib.sha256(Path(common.__file__).read_bytes()).hexdigest(),
              'codexVersion': subprocess.check_output([str(args.codex), '--version'], text=True).strip()}
    runtime = surface = codex = None
    try:
        with tempfile.TemporaryDirectory(prefix='art-term-') as directory:
            try:
                runtime = common.Runtime(args, Path(directory))
                surface = WindowsSurface(runtime)
                result['binarySha256'] = surface.windows.sha
                result['windowsRoot'] = surface.windows.root
                result['binding'] = surface.binding
                runtime.bindings.add(surface.binding)
                codex = common.Codex(runtime, surface, args.case, args.output_dir)
                common.run_case(runtime, surface, codex, result)
                result.update(process=codex.identity(), sessionId=codex.session_id(), passed=True)
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
        result.update(passed=False, error=str(error))
    (args.output_dir / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result), flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
