"""Unified connection management through an owned APK, native IPC and real touches."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import tomllib
import uuid

from aiohttp import web
from cdp import App
from device import PACKAGE, ROOT, require, wait
from linux_emulator import LinuxEmulator
from run import Fixture


def row_selector(identity):
    return '[data-connection-id=' + json.dumps(identity) + ']'


class Connections:
    def __init__(self, device, fixture, output):
        self.device, self.fixture, self.output = device, fixture, output
        self.app = None

    def launch(self):
        self.device.start()
        self.app = App(self.device).connect()
        wait(lambda: self.app.has('.settings-trigger'))
        self.app.evaluate('''(()=>{window.__connectionPointers=[];window.__connectionReorders=[];
for(const type of ['pointerdown','pointermove','pointerup','pointercancel'])
document.addEventListener(type,e=>{const hit=document.elementFromPoint(e.clientX,e.clientY)?.closest('[data-connection-id]');
window.__connectionPointers.push({type,trusted:e.isTrusted,x:e.clientX,y:e.clientY,
button:e.button,isPrimary:e.isPrimary,pointerId:e.pointerId,pointerType:e.pointerType,
target:e.target.tagName,disabled:e.target.closest('button')?.disabled,
hit:hit?.dataset.connectionId,kind:hit?.dataset.connectionKind,
dragging:document.querySelector('.dragging')?.dataset.connectionId,
before:document.querySelector('.drop-before')?.dataset.connectionId,
after:document.querySelector('.drop-after')?.dataset.connectionId});
window.__connectionPointers=window.__connectionPointers.slice(-100)},true);
const native=window.ipc.postMessage.bind(window.ipc),callbacks=window.__TAURI_INTERNALS__.callbacks;
window.ipc={postMessage:data=>{const message=JSON.parse(data);
if(message.cmd==='gui_runtime_reorder_connections'){
window.__connectionReorders.push({phase:'request',...message.payload});
for(const [key,phase]of [['callback','success'],['error','error']]){const callback=callbacks.get(message[key]);
if(callback)callbacks.set(message[key],value=>{const detail=phase==='error'?(typeof value==='string'?value:JSON.stringify(value)).slice(0,512):undefined;
window.__connectionReorders.push({phase,detail});return callback(value)})}}
return native(data)}}})()''')

    def restart(self):
        self.app.close()
        self.device.stop()
        self.launch()

    def settings(self):
        self.app.click('.settings-trigger')
        wait(lambda: self.app.has('[data-testid=runtime-create]'))
        require(not self.app.has('[data-testid=runtime-host-form]'), 'creation form visible before add')

    def stored(self):
        return tomllib.loads(self.device.store().decode('utf-8-sig'))

    def save(self, kind, name, url, token, display=None):
        app = self.app
        app.tap('runtime-create')
        wait(lambda: app.has('[data-testid=runtime-host-form]'))
        require(app.evaluate('[...document.querySelectorAll("input[name=connection-kind]")].map(e=>e.value)') == ['host', 'agent', 'screen'],
                'connection types are not three shared radio choices')
        app.tap('connection-type-' + kind)
        app.set('name', name)
        app.set('base-url', url)
        if display is not None:
            app.set('display-name', display)
        app.set('credential', token)
        app.click('[data-testid=runtime-host-form] button[type=submit]')
        wait(lambda: not app.has('[data-testid=runtime-host-form]'))
        require('儲存成功' in app.text('[data-testid=connection-toast]'), 'saved result toast missing')
        values = self.stored()['desktops' if kind == 'screen' else 'connections']
        saved = next(item for item in values if item['name'] == name)
        require(app.has(row_selector(saved['id'])), 'saved row missing')
        return saved

    def action(self, identity, action):
        self.app.click(row_selector(identity) + ' [data-action=' + action + ']')

    def seed(self, kind, name, url, token, display=None):
        if kind == 'screen':
            command = 'gui_instance_save'
            value = {'name': name, 'mcpUrl': url, 'token': token}
        else:
            command = 'gui_runtime_save_host'
            value = {'name': name, 'baseUrl': url, 'credential': token, 'category': kind}
            if display is not None:
                value['displayName'] = display
        return self.app.evaluate('window.__TAURI_INTERNALS__.invoke(' + json.dumps(command)
                                 + ',' + json.dumps({'input': value}) + ')')

    def edit_alias(self, identity, value):
        self.action(identity, 'edit')
        require(self.app.evaluate('document.querySelector("[data-testid=credential]").value') == '', 'editing disclosed saved credential')
        self.app.set('display-name', value)
        self.app.click('[data-testid=runtime-host-form] button[type=submit]')
        wait(lambda: not self.app.has('[data-testid=runtime-host-form]'))

    def drag(self, source, target):
        app = self.app
        source_css = row_selector(source) + ' [data-action=reorder]'
        target_css = row_selector(target)
        app.evaluate(f'document.querySelector({json.dumps(source_css)}).scrollIntoView({{block:"center"}})')
        time.sleep(.25)
        points = app.evaluate(f'''(()=>{{const a=document.querySelector({json.dumps(source_css)}).getBoundingClientRect(),
b=document.querySelector({json.dumps(target_css)}).getBoundingClientRect();return {{sx:a.x+a.width/2,sy:a.y+a.height/2,
tx:b.x+18,ty:b.y+b.height*.2,vw:innerWidth}}}})()''')
        bounds = self.device.webview_bounds()
        scale = (bounds[2] - bounds[0]) / points['vw']
        coordinates = [round(bounds[0] + points['sx'] * scale), round(bounds[1] + points['sy'] * scale),
                       round(bounds[0] + points['tx'] * scale), round(bounds[1] + points['ty'] * scale)]
        require(all(bounds[0] <= x < bounds[2] for x in coordinates[::2])
                and all(bounds[1] <= y < bounds[3] for y in coordinates[1::2]), 'drag endpoints outside WebView')
        app.evaluate('window.__connectionLastDrag=' + json.dumps(points))
        app.evaluate('window.__connectionPointers=[]')
        self.device.call('shell', 'input', 'swipe', *coordinates, 700)
        wait(lambda: app.evaluate('window.__connectionPointers.some(e=>e.type==="pointerup"&&e.trusted)'))

    def page(self, name):
        self.app.click_text(name, '.page-text-navigation button')
        wait(lambda: self.current(name))

    def current(self, name):
        return self.app.evaluate(f'[...document.querySelectorAll(".page-text-navigation button")].some(e=>e.textContent.trim()==={json.dumps(name)}&&e.getAttribute("aria-current")==="page")')

    def swipe(self, selector, right):
        rect = self.app.evaluate(f'(()=>{{const r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return {{x:r.x,y:r.y,w:r.width,h:r.height,vw:innerWidth}}}})()')
        bounds = self.device.webview_bounds()
        scale = (bounds[2] - bounds[0]) / rect['vw']
        y = round(bounds[1] + (rect['y'] + rect['h'] * .6) * scale)
        left = round(bounds[0] + (rect['x'] + rect['w'] * .15) * scale)
        edge = round(bounds[0] + (rect['x'] + rect['w'] * .85) * scale)
        self.device.call('shell', 'input', 'swipe', left if right else edge, y, edge if right else left, y, 420)

    def visible(self, selector):
        return self.app.evaluate(f'''(()=>{{const e=document.querySelector({json.dumps(selector)});if(!e)return false;
const r=e.getBoundingClientRect();return !!r.width&&!!r.height&&r.right>0&&r.left<innerWidth&&getComputedStyle(e).display!=="none"}})()''')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', required=True, type=Path)
    parser.add_argument('--source-sha', required=True)
    parser.add_argument('--seed-extra-via-ipc', action='store_true',
                        help='Create the first Computer through UI; seed other records through real native save commands.')
    parser.add_argument('--sdk', type=Path, default=Path('/home/art/Android/Sdk'))
    parser.add_argument('--output', type=Path, default=Path(tempfile.gettempdir()) / ('art-term-connections-' + uuid.uuid4().hex))
    args = parser.parse_args()
    output = args.output.resolve()
    result = {'passed': False, 'sourceSha': args.source_sha, 'apkSha256': hashlib.sha256(args.apk.read_bytes()).hexdigest(),
              'realApk': True, 'mockedIpc': False, 'cases': {}, 'cleanup': {},
              'limits': ['Owned x86_64 emulator, not a physical ARM64 phone.',
                         'Local TLS Runtime and MCP fixtures, not a real AI CLI or production network.']}
    device = fixture = suite = None
    screens = []
    stage = 'boot'
    print('OUTPUT ' + str(output), flush=True)
    try:
        device = LinuxEmulator(args.sdk, output)
        device.start_emulator()
        device.install(args.apk.resolve())
        installed = device.text('shell', 'pm', 'path', PACKAGE).removeprefix('package:')
        require(device.text('shell', 'sha256sum', installed).split()[0] == result['apkSha256'], 'installed APK hash mismatch')
        fixture = Fixture(output)
        origins = [fixture.origin]
        async def extra_origin():
            site = web.TCPSite(fixture.runner, '127.0.0.1', 0, ssl_context=fixture.context)
            await site.start()
            return 'https://127.0.0.1:' + str(site._server.sockets[0].getsockname()[1])
        origins.extend(fixture.call(extra_origin()) for _ in range(2))
        for origin in origins:
            device.reverse(int(origin.rsplit(':', 1)[1]))
        device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der', fixture.ca.read_bytes())
        spec = importlib.util.spec_from_file_location('connection_mcp_fixture', ROOT / 'scripts/instance-mcp-regression/fixture.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        screens = [module.Fixture('A'), module.Fixture('B', sse=True)]
        for screen in screens:
            device.reverse(screen.server.server_port)
        suite = Connections(device, fixture, output)
        suite.launch()
        stage = 'unified-crud'
        suite.settings()
        computer = suite.save('host', 'Computer', origins[0], fixture.credential)
        create = suite.seed if args.seed_extra_via_ipc else suite.save
        agent_a = create('agent', 'Agent A', origins[1], fixture.credential, '中文開發助手')
        agent_b = create('agent', 'Agent B', origins[2], fixture.credential)
        screen_a = create('screen', 'Screen A', screens[0].url, screens[0].token)
        screen_b = create('screen', 'Screen B', screens[1].url, screens[1].token)
        if args.seed_extra_via_ipc:
            suite.page('Main')
            suite.settings()
        raw = device.store()
        require(all(secret.encode() not in raw for secret in [fixture.credential, *(screen.token for screen in screens)]), 'plaintext token in settings')
        stored = suite.stored()
        require(all(item.get('credential_ciphertext') for item in stored['connections'])
                and all(item.get('token_ciphertext') for item in stored['desktops']), 'native encrypted credential missing')
        require(stored['connections'][1]['display_name'] == '中文開發助手', 'Chinese alias absent from native store')
        result['cases'][stage] = {'passed': True, 'types': 3, 'hosts': 3, 'screens': 2, 'encryptedTokens': True,
                                 'uiCreated': 1 if args.seed_extra_via_ipc else 5,
                                 'ipcCreated': 4 if args.seed_extra_via_ipc else 0}
        print('PASS ' + stage, flush=True)
        device.screenshot(output / 'unified-list.png')
        stage = 'native-connection-test-toast'
        suite.action(computer['id'], 'test')
        wait(lambda: '連線測試成功' in suite.app.text('[data-testid=connection-toast]'))
        require('成功' not in suite.app.text(row_selector(computer['id'])), 'result message leaked into row')
        wait(lambda: not suite.app.has('[data-testid=connection-toast]'), 6)
        suite.action(screen_a['id'], 'test')
        wait(lambda: '連線測試成功' in suite.app.text('[data-testid=connection-toast]'))
        require(any(item['method'] == 'initialize' for item in screens[0].events), 'Screen test never reached actual MCP fixture')
        result['cases'][stage] = {'passed': True, 'runtimeHttps': True, 'screenMcp': True, 'toastExpires': True}
        print('PASS ' + stage, flush=True)
        stage = 'native-touch-reorder'
        suite.drag(agent_b['id'], agent_a['id'])
        expected_hosts = [computer['id'], agent_b['id'], agent_a['id']]
        wait(lambda: [item['id'] for item in suite.stored()['connections']] == expected_hosts)
        suite.drag(screen_b['id'], screen_a['id'])
        expected_screens = [screen_b['id'], screen_a['id']]
        wait(lambda: [item['id'] for item in suite.stored()['desktops']] == expected_screens)
        before = device.store()
        suite.drag(agent_a['id'], screen_b['id'])
        time.sleep(.5)
        require(device.store() == before, 'cross-type drag mutated native settings')
        require(suite.app.has('.global-settings-page'), 'sorting triggered workspace navigation')
        result['cases'][stage] = {'passed': True, 'agents': True, 'screens': True, 'crossTypeRejected': True, 'noPagerGesture': True}
        print('PASS ' + stage, flush=True)
        stage = 'restart-navigation-alias'
        suite.restart()
        require([item['id'] for item in suite.app.profiles()] == expected_hosts, 'native ordering changed after restart')
        suite.page('Main')
        wait(lambda: suite.app.has('[data-testid="runtime-' + agent_a['id'] + '"]'))
        require(suite.app.evaluate('[...document.querySelectorAll("[data-testid=cat-agents] .host-connections")].map(e=>e.dataset.testid)')
                == ['runtime-' + agent_b['id'], 'runtime-' + agent_a['id']], 'Agent navigation order mismatch')
        require(suite.app.evaluate('[...document.querySelectorAll("[data-testid=cat-screens] .saved-screen-row")].map(e=>e.dataset.testid)')
                == ['screen-' + screen_b['id'], 'screen-' + screen_a['id']], 'Screen navigation order mismatch')
        require('中文開發助手' in suite.app.text('[data-testid="runtime-' + agent_a['id'] + '"]'), 'Chinese alias absent from navigation')
        suite.settings()
        require([item['id'] for item in suite.stored()['desktops']] == expected_screens, 'Screen ordering changed after restart')
        before = device.store()
        suite.action(agent_a['id'], 'edit')
        suite.app.set('display-name', '漢' * 21)
        suite.app.click('[data-testid=runtime-host-form] button[type=submit]')
        require(suite.app.has('[data-testid=runtime-host-form]') and '20' in suite.app.text('[data-testid=form-error]'), '21-character alias was not refused')
        require(device.store() == before, 'invalid alias changed native settings')
        suite.app.set('display-name', '漢' * 20)
        suite.app.click('[data-testid=runtime-host-form] button[type=submit]')
        wait(lambda: not suite.app.has('[data-testid=runtime-host-form]'))
        suite.edit_alias(agent_a['id'], '')
        profile = next(item for item in suite.app.profiles() if item['id'] == agent_a['id'])
        require(not profile.get('displayName'), 'blank alias did not restore default')
        result['cases'][stage] = {'passed': True, 'orderRetained': True, 'chineseAlias': True, 'twentyCharacterBoundary': True, 'emptyAliasDefault': True}
        print('PASS ' + stage, flush=True)
        stage = 'delete-screen'
        suite.action(screen_a['id'], 'delete')
        suite.app.tap('confirm-delete')
        wait(lambda: not suite.app.has(row_selector(screen_a['id'])))
        require(len(suite.stored()['connections']) == 3 and len(suite.stored()['desktops']) == 1, 'Screen delete changed Host records')
        result['cases'][stage] = {'passed': True, 'hostRecordsPreserved': True}
        print('PASS ' + stage, flush=True)
        stage = 'mobile-homepage-pager'
        suite.page('Main')
        suite.swipe('.home-surface', True)
        wait(lambda: suite.visible('.software-home-page'))
        suite.swipe('.software-home-page', False)
        wait(lambda: suite.current('Main'))
        suite.swipe('.home-surface', False)
        wait(lambda: suite.current('Connect'))
        suite.swipe('#terminal-panel', True)
        wait(lambda: suite.current('Main'))
        suite.app.click('.fixed-brand')
        wait(lambda: suite.visible('.software-home-page'))
        require(suite.visible('.workspace-header'), 'header vanished during pager navigation')
        result['cases'][stage] = {'passed': True, 'homeMainConnectSwipes': True, 'brandUsesPagerPage': True}
        print('PASS ' + stage, flush=True)
        stage = 'manual-terminal-command'
        suite.page('Main')
        suite.app.click('[data-testid="runtime-' + computer['id'] + '"] .new-connection')
        wait(lambda: suite.app.evaluate('Object.keys(window.__reg.sessions).length') == 1, 40)
        wait(lambda: suite.current('Connect'))
        require(not suite.app.has('[data-testid=launch-coding-cli]'), 'dedicated CLI launch button remains')
        suite.app.tap('k-composer')
        suite.app.set('composer-text', 'codex --help')
        suite.app.tap('composer-send-enter')
        wait(lambda: any('codex --help\r' in item['input'] for item in fixture.snapshot()['sessions'].values()))
        result['cases'][stage] = {'passed': True, 'launchButtonAbsent': True, 'typedCommandReachedNativeRuntime': True,
                                 'fixtureDoesNotExecuteCli': True}
        print('PASS ' + stage, flush=True)
        result['webview'] = suite.app.metadata
        result['passed'] = True
    except Exception as error:
        result['failure'] = {'stage': stage, 'type': type(error).__name__, 'message': str(error),
                             'action': suite.app.action if suite and suite.app else None}
        if suite and suite.app:
            try:
                result['failure']['touch'] = getattr(suite.app, 'touch_diagnostic', None)
                result['failure']['pointers'] = suite.app.evaluate('window.__connectionPointers')
                result['failure']['drag'] = suite.app.evaluate('window.__connectionLastDrag')
                result['failure']['reorders'] = suite.app.evaluate('window.__connectionReorders')
                html = suite.app.evaluate('document.documentElement.outerHTML')
                for secret in [fixture.credential, *(screen.token for screen in screens)]:
                    html = html.replace(secret, '[redacted]')
                (output / 'failure-dom.html').write_text(html)
                device.screenshot(output / 'failure.png')
            except Exception:
                result['failure']['diagnosticsFailed'] = True
        print('FAIL ' + stage + ' ' + type(error).__name__, flush=True)
    finally:
        cleanups = [('cdp', suite.app.close if suite and suite.app else None),
                    *((f'mcp-{index}', screen.close) for index, screen in enumerate(screens)),
                    ('runtime', fixture.close if fixture else None),
                    ('device', device.cleanup if device else None), ('emulator', device.shutdown if device else None)]
        for name, cleanup in cleanups:
            if cleanup:
                try:
                    cleanup()
                    result['cleanup'][name] = True
                except Exception:
                    result['cleanup'][name] = False
                    result['passed'] = False
        output.mkdir(parents=True, exist_ok=True)
        (output / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print('PASS' if result['passed'] else 'FAIL', flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
