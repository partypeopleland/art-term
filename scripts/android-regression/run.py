"""Bounded true APK E2E on an explicitly root-owned emulator, no fake IPC."""
import argparse
import hashlib
import json
import re
import secrets
import subprocess
import sys
import time
import tomllib
import uuid
import _thread
import threading
from pathlib import Path
from device import Device, PACKAGE, STORE, ROOT, require, wait
from cdp import App

sys.path.insert(0, str(ROOT / 'scripts/windows-regression'))
from fixture import Fixture


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def safe_output(value):
    raw = Path(value).absolute()
    require(raw.resolve().is_relative_to((ROOT / 'target/android-regression').resolve()), 'output outside Android task target')
    for p in [raw, *raw.parents]:
        require(not p.is_symlink() and not p.is_junction(), 'output reparse point rejected')
        if p == ROOT:
            break
    raw.mkdir(parents=True, exist_ok=True)
    return raw.resolve()


class Suite:
    def __init__(self, device, fixture, apk, run):
        self.device, self.fixture, self.apk, self.run = device, fixture, apk, run
        self.app = None

    def launch(self):
        self.device.start()
        self.app = App(self.device).connect()
        wait(lambda: self.app.has('[data-testid=runtime-host-manage]'))
        return self.app

    def restart(self):
        self.app.close()
        self.device.stop()
        return self.launch()

    def test_host(self):
        self.app.click_text('測試', '[data-host-id] button')
        wait(lambda: '已通過管理認證' in self.app.text('[data-host-id] [role=status]'))

    def add(self):
        a = self.app
        a.tap('runtime-create')
        a.set('name', 'Android fixture')
        a.set('base-url', self.fixture.origin)
        a.set('credential', self.fixture.credential)
        a.click_text('儲存', '[data-testid=runtime-host-form] button')
        wait(lambda: len(a.profiles()) == 1)
        return a.profiles()[0]

    def crud(self):
        a = self.app
        require(a.profiles() == [], 'new owned APK has existing profiles')
        a.manage()
        profile = self.add()
        self.test_host()
        a.click_text('編輯', '[data-host-id] button')
        a.set('name', 'Android renamed')
        require(a.evaluate('document.querySelector("[data-testid=credential]").value') == '', 'edit disclosed credential')
        a.evaluate('document.querySelector("[data-testid=runtime-host-category]").focus()')
        a.key('ArrowDown', 'ArrowDown')
        a.key('Enter', 'Enter')
        a.click_text('儲存', '[data-testid=runtime-host-form] button')
        wait(lambda: a.profiles()[0]['revision'] == profile['revision'] + 1)
        renamed = a.profiles()[0]
        require(renamed['id'] == profile['id'] and renamed['category'] == 'agent', 'edit category or identity mismatch')
        raw = self.device.store()
        doc = tomllib.loads(raw.decode('utf-8-sig'))
        require(doc['schema_version'] == 5 and doc['connections'][0]['credential_ciphertext'], 'schema5 sealed credential absent')
        require(self.fixture.credential.encode() not in raw, 'plaintext credential in Android store')
        a = self.restart()
        require(a.profiles() == [renamed], 'Keystore restart profile changed')
        a.manage()
        self.test_host()
        a.click_text('刪除', '[data-host-id] button')
        a.tap('confirm-delete')
        wait(lambda: a.profiles() == [])
        a = self.restart()
        require(a.profiles() == [], 'last deleted Host reappeared')
        a.manage()
        self.add()
        # Supported public metadata only; preserve the ciphertext and native identity.
        a.close()
        self.device.stop()
        raw = self.device.store().decode('utf-8-sig')
        raw = raw.replace('category = "host"', 'category = "host"\ndesktop_transport = "runtime-mcp"\ncoding_cli = { id = "fixture-cli", name = "Fixture CLI", command = "fixture-cli", args = ["--literal", "a b"] }')
        self.device.private_write(STORE, raw.encode())
        self.launch()
        return {'uiAddTestEditDelete': True, 'schema5KeystoreRestart': True, 'lastDeleteRemainsEmpty': True, 'blankCredentialPreserved': True}

    def shell(self, name):
        a = self.app
        if a.has('[data-testid=to-list]') and a.evaluate('document.querySelector("[data-testid=to-list]").getBoundingClientRect().width>0'):
            a.tap('to-list')
        profile = a.profiles()[0]
        a.tap('runtime-shell-' + profile['id'])
        a.fill('.runtime-sessions form input', name)
        a.click_text('建立並連線', '.runtime-sessions form button')
        wait(lambda: a.text('.tab.active .name') == name)
        wait(lambda: 'live' in a.evaluate('document.querySelector("[data-testid=link-status]")?.className||""'))

    def compose(self, text):
        self.app.tap('k-composer')
        self.app.set('composer-text', text)
        self.app.tap('composer-send-enter')

    def contains(self, name, text):
        return self.app.evaluate(f'Object.values(window.__reg.sessions).some(s=>s.name==={json.dumps("openab:"+name)}&&s.tail.includes({json.dumps(text)}))')

    def tab(self, name):
        self.app.click_text(name, '.tab .name')

    def terminal(self):
        a, f = self.app, self.fixture
        self.shell('work-a')
        self.compose('UNICODE_繁體測試')
        wait(lambda: self.contains('work-a', 'UNICODE_繁體測試'))
        wait(lambda: a.evaluate('window.__reg.acks>0'))
        require(a.evaluate('[...document.querySelectorAll(".xterm-rows")].some(e=>e.textContent.includes("UNICODE_繁體測試"))'), 'Unicode not rendered in native xterm')
        a.tap('k-left')
        a.tap('k-ctrlc')
        wait(lambda: '\x1b[D' in f.snapshot()['sessions']['work-a']['input'] and '\x03' in f.snapshot()['sessions']['work-a']['input'])
        a.tap('launch-coding-cli')
        wait(lambda: "'fixture-cli' '--literal' 'a b'\r" in f.snapshot()['sessions']['work-a']['input'])
        self.shell('work-b')
        self.compose('SECOND_TAB')
        wait(lambda: self.contains('work-b', 'SECOND_TAB'))
        require(not self.contains('work-a', 'SECOND_TAB') and not self.contains('work-b', 'UNICODE_繁體測試'), 'tab outputs crossed')
        self.tab('work-a')
        before_size = f.snapshot()['sessions']['work-a']['resizes']
        self.device.call('shell', 'settings', 'put', 'system', 'accelerometer_rotation', '0')
        self.device.call('shell', 'settings', 'put', 'system', 'user_rotation', '1')
        wait(lambda: a.evaluate('visualViewport.width>visualViewport.height') and f.snapshot()['sessions']['work-a']['resizes'] != before_size)
        before_size = f.snapshot()['sessions']['work-a']['resizes']
        self.device.call('shell', 'settings', 'put', 'system', 'user_rotation', '0')
        wait(lambda: a.evaluate('visualViewport.height>visualViewport.width') and f.snapshot()['sessions']['work-a']['resizes'] != before_size)
        wait(lambda: a.evaluate('window.__reg.acks>2'))
        before = f.snapshot()
        f.disconnect('work-a')
        count = len(before['sessions']['work-a']['attaches'])
        wait(lambda: len(f.snapshot()['sessions']['work-a']['attaches']) == count + 1, 30)
        after = f.snapshot()
        old, new = after['sessions']['work-a']['attaches'][-2:]
        require(old['tokenHash'] == new['tokenHash'], 'recovery minted token')
        require(new['since'] == before['sessions']['work-a']['historyBytes'] and new['since'] > 0, 'recovery cursor mismatch')
        require(after['sessions']['work-a']['input'] == before['sessions']['work-a']['input'] and after['events'] == before['events'], 'recovery replay or session mutation')
        self.compose('AFTER_RECOVERY')
        wait(lambda: self.contains('work-a', 'AFTER_RECOVERY'))
        self.tab('work-b')
        a.click('.tab.active button.close')
        wait(lambda: not f.snapshot()['sessions']['work-b']['attached'])
        require('work-b' in f.snapshot()['sessions'], 'close deleted remote Shell')
        self.tab('work-a')
        return {'realXtermUnicode': True, 'touchKeyBytes': True, 'nativeRotationResize': True, 'tabsIsolated': True, 'sameTokenCursorRecovery': True, 'noReplay': True, 'closeDetachesOnly': True}

    def rendered(self):
        return self.app.evaluate('(()=>{const i=document.querySelector("img.shot");return !!i&&i.complete&&i.naturalWidth===512&&i.naturalHeight===256})()')

    def shots(self):
        return self.app.evaluate('window.__reg.shots')

    def preview(self):
        a, f = self.app, self.fixture
        started = time.monotonic()
        a.tap('desktop-toggle')
        wait(lambda: self.shots() >= 1 and self.rendered(), 25)
        first = self.shots()
        first_elapsed = time.monotonic() - started
        require(first_elapsed < 20, 'first preview waited for periodic tick')
        wait(lambda: self.shots() >= first + 2 and self.rendered(), 75)
        starts = a.evaluate('window.__reg.shotStarts')
        interval = (starts[-1]-starts[-2])/1000
        require(24 <= interval <= 38, 'production preview interval mismatch')
        before = self.shots()
        a.click_text('⟳ 更新', '.preview button')
        wait(lambda: self.shots() > before and self.rendered(), 25)
        # Reopen a second retained Shell through its existing native UI.
        a.tap('to-list')
        a.tap('runtime-shell-' + a.profiles()[0]['id'])
        wait(lambda: a.has('.runtime-sessions li'))
        # Existing work-b is the second row, as creation order is stable in fixture.
        index = wait(lambda:a.evaluate('[...document.querySelectorAll(".runtime-sessions li")].findIndex(e=>e.querySelector("strong").textContent.trim()==="work-b")+1'))
        a.click(f'.runtime-sessions li:nth-child({index}) button')
        wait(lambda: a.text('.tab.active .name') == 'work-b')
        require(not self.rendered(), 'preview crossed tabs')
        self.tab('work-a')
        a.tap('desktop-toggle')
        wait(lambda: not a.has('img.shot'))
        wait(lambda: not any(n.startswith('mcshot-') for n in f.snapshot()['sessions']))
        state = f.snapshot()
        require(all('preview.py' not in s['input'] and 'ARTTERM_PREVIEW' not in s['input'] for n,s in state['sessions'].items() if n.startswith('work-')), 'preview polluted work Shell')
        deleted = [e['name'] for e in state['events'] if e['method']=='DELETE']
        created = [e['name'] for e in state['events'] if e['method']=='POST' and e.get('action') is None and e['name'].startswith('mcshot-')]
        require(deleted and sorted(deleted)==sorted(created), 'preview cleanup ownership mismatch')
        return {'pngBytes':state['pngBytes'],'actualWebViewImageDecoded':True,'firstSeconds':round(first_elapsed,2),'productionIntervalSeconds':round(interval,2),'manual':True,'tabIsolation':True,'ownedPreviewShellCleanup':True}

    def lifecycle(self):
        a, f = self.app, self.fixture
        self.tab('work-a')
        a.tap('desktop-toggle')
        wait(self.rendered, 25)
        a.tap('k-ctrl')
        a.tap('k-composer')
        a.set('composer-text','MUST_NOT_REPLAY_BACKGROUND')
        before = f.snapshot()['sessions']['work-a']
        shots = self.shots()
        self.device.key('KEYCODE_HOME')
        wait(lambda: a.evaluate('document.hidden'))
        # Hold across a production 30s refresh tick.
        time.sleep(33)
        require(self.shots()==shots, 'background captured preview')
        self.device.start()
        wait(lambda: not a.evaluate('document.hidden'))
        wait(lambda: a.evaluate('!document.querySelector("[data-testid=composer]")'))
        require(a.evaluate('!document.querySelector("[data-testid=k-ctrl]").classList.contains("on")'), 'background did not clear Ctrl')
        after = f.snapshot()['sessions']['work-a']
        require(after['input']==before['input'], 'background replayed composer input')
        require(after['attaches'][-1]['tokenHash']==before['attaches'][-1]['tokenHash'], 'resume changed binding token')
        a.tap('k-composer')
        require(a.evaluate('document.querySelector("[data-testid=composer-text]").value') == '', 'composer text survived background')
        a.tap('composer-cancel')
        self.compose('AFTER_NATIVE_RESUME')
        wait(lambda: self.contains('work-a','AFTER_NATIVE_RESUME'))
        a.tap('desktop-toggle')
        wait(lambda: not any(n.startswith('mcshot-') for n in f.snapshot()['sessions']))
        return {'nativeHomePauseResume':True,'backgroundPreviewStoppedSeconds':33,'composerAndCtrlCleared':True,'originalBinding':True,'noReplay':True,'notDozeOrOemProof':True}

    def export(self, filename, password):
        a = self.app
        filename = self.run.name[4:] + '-' + filename
        a.tap('runtime-export')
        a.click('.runtime-transfer fieldset input[type=checkbox]')
        a.set('transfer-password',password)
        a.set('transfer-confirm',password)
        a.click_text('選擇位置並儲存','.runtime-transfer button')
        self.device.save_document(filename)
        wait(lambda: '連線檔案已儲存' in a.text('[data-testid=runtime-transfer]'), 30)
        path = self.run / filename
        self.device.call('pull','/sdcard/Download/'+filename,str(path))
        raw = path.read_text()
        require(json.loads(raw)['mode']=='encrypted' and self.fixture.credential not in raw and self.fixture.origin not in raw, 'SAF export leaked plaintext')
        return path

    def pick_import(self, filename, password):
        a = self.app
        a.tap('transfer-pick')
        self.device.open_document(filename)
        wait(lambda: a.has('[data-testid=transfer-password]'))
        a.set('transfer-password',password)
        a.click_text('解鎖並預覽','.runtime-transfer button')

    def saf(self):
        a = self.app
        a.manage()
        password = secrets.token_urlsafe(24)
        exported = self.export('android-export.json',password)
        a.click_text('返回管理','.runtime-transfer button')
        original = a.profiles()[0]
        a.click_text('刪除','[data-host-id] button')
        a.tap('confirm-delete')
        wait(lambda: a.profiles()==[])
        a.tap('runtime-import')
        a.tap('transfer-pick')
        self.device.documents()
        self.device.key('KEYCODE_BACK')
        wait(lambda: '已取消選檔' in a.text('[data-testid=runtime-transfer]'))
        before = self.device.store()
        self.pick_import(exported.name,'wrong-password')
        wait(lambda: '密碼錯誤或檔案已損壞' in a.text('[data-testid=runtime-transfer]'),30)
        require(self.device.store()==before and a.profiles()==[], 'wrong password wrote destination')
        self.pick_import(exported.name,password)
        a.tap('transfer-apply')
        wait(lambda: len(a.profiles())==1,30)
        restored = a.profiles()[0]
        require(restored['id']!=original['id'] and restored['credentialPresent'], 'SAF did not create fresh credential identity')
        a.click_text('返回管理','.runtime-transfer button')
        self.test_host()
        a.tap('runtime-import')
        before = self.device.store()
        self.pick_import(exported.name,password)
        wait(lambda: a.has('.preview-list input[type=checkbox]'))
        require(a.evaluate('!document.querySelector(".preview-list input[type=checkbox]").checked&&document.querySelector("[data-testid=transfer-apply]").disabled'), 'duplicate not default skip')
        require(self.device.store()==before, 'duplicate preview wrote store')
        a.click('.preview-list input[type=checkbox]')
        a.tap('transfer-apply')
        wait(lambda: a.profiles()[0]['revision']==restored['revision']+1,30)
        a = self.restart()
        a.manage()
        self.test_host()
        require(a.profiles()[0]['id']==restored['id'], 'SAF update identity lost on restart')
        return {'nativeSafExportImportCancel':True,'encrypted':True,'wrongPasswordNoWrite':True,'freshKeystoreIdentity':True,'duplicateDefaultSkip':True,'explicitUpdateRestart':True}

    def update(self):
        before, profiles = self.device.store(), self.app.profiles()
        self.app.close()
        self.device.stop()
        self.device.install(self.apk,replace=True)
        self.launch()
        require(self.device.store()==before and self.app.profiles()==profiles,'install-r lost store or identity')
        self.app.manage()
        self.test_host()
        return {'sameDebugApkInstallR':True,'schema5AndKeystoreCredentialPreserved':True,'notHistoricalMigrationProof':True}

    def cross_transfer(self, exe, driver):
        from webview import App as WindowsApp
        from selenium.webdriver.common.by import By
        wrun = ROOT / 'target/windows-regression' / ('run-'+uuid.uuid4().hex)
        wrun.mkdir(parents=True)
        win = WindowsApp(Path(exe).resolve(strict=True),Path(driver).resolve(strict=True),wrun,self.fixture)
        password = secrets.token_urlsafe(24)
        try:
            win.launch(wrun/'source.toml')
            win.manage()
            win.node('runtime-create').click()
            for field,value in [('name','Windows transfer'),('base-url',self.fixture.origin),('credential',self.fixture.credential)]:
                win.fill(field,value)
            win.click('儲存','[data-testid=runtime-host-form] button')
            win.wait(lambda: len(win.profiles())==1)
            source = win.profiles()[0]
            win.node('runtime-export').click()
            win.wait(lambda: win.find('.runtime-transfer fieldset input[type=checkbox]'))[0].click()
            win.fill('transfer-password',password)
            win.fill('transfer-confirm',password)
            exported = wrun/'windows-export.json'
            win.picker(lambda:win.click('選擇位置並儲存','.runtime-transfer button'),exported)
            win.wait(exported.exists)
            raw = exported.read_text()
            require(json.loads(raw)['mode']=='encrypted' and self.fixture.credential not in raw and self.fixture.origin not in raw, 'Windows cross export leaked plaintext')
            win.close()
            self.app.close()
            self.device.stop()
            self.device.clear()
            self.device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der',self.fixture.ca.read_bytes())
            self.device.call('push',str(exported),'/sdcard/Download/windows-export.json')
            a = self.launch()
            a.manage()
            a.tap('runtime-import')
            self.pick_import('windows-export.json',password)
            a.tap('transfer-apply')
            wait(lambda:len(a.profiles())==1,30)
            require(a.profiles()[0]['id']!=source['id'] and a.profiles()[0]['credentialPresent'],'Windows to Android identity not resealed')
            a = self.restart()
            a.manage()
            self.test_host()
            aexport = self.export('android-cross-export.json',password)
            local = wrun/'android-export.json'
            local.write_bytes(aexport.read_bytes())
            win.launch(wrun/'destination.toml')
            win.manage()
            win.node('runtime-import').click()
            win.picker(lambda:win.node('transfer-pick').click(),local)
            win.fill('transfer-password',password)
            win.click('解鎖並預覽','.runtime-transfer button')
            win.node('transfer-apply').click()
            win.wait(lambda:len(win.profiles())==1,30)
            restored = win.profiles()[0]
            require(restored['id']!=a.profiles()[0]['id'] and restored['credentialPresent'],'Android to Windows identity not resealed')
            win.close()
            win.launch(wrun/'destination.toml')
            win.manage()
            win.click('測試','[data-host-id] button')
            win.wait(lambda:any('已通過管理認證' in e.text for e in win.find('[data-host-id] [role=status]')))
            return {'windowsNativePickerToAndroidSaf':True,'androidSafToWindowsNativePicker':True,'bothFreshLocalCredentialIdentities':True,'bothNativeAuthenticationAfterRestart':True,'windowsRun':str(wrun),
                    'windowsExeSha256':sha(exe),'edgeDriverSha256':sha(driver),'windowsWebview':win.metadata}
        finally:
            win.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ('adb','serial','apk','owner-manifest','windows-exe','edge-driver'):
        parser.add_argument('--' + option, required=True)
    parser.add_argument('--output-root', type=Path, default=ROOT / 'target/android-regression')
    args = parser.parse_args()
    run = safe_output(args.output_root) / ('run-' + uuid.uuid4().hex)
    run.mkdir()
    result = {'passed': False, 'actualApk': True, 'mockedIpcReplies': False, 'touchTransport':'native-adb-input-tap', 'backend': 'protocolfixture', 'cases': {}, 'cleanup': {}}
    device = fixture = suite = None
    timer = threading.Timer(1100, _thread.interrupt_main)
    timer.daemon = True
    timer.start()
    stage = 'preflight'
    try:
        apk = Path(args.apk).resolve(strict=True)
        result['apkSha256'] = sha(apk)
        source = sorted([*ROOT.glob('crates/art-term-core/src/**/*.rs'), *ROOT.glob('crates/art-term-gui/src/**/*.rs'), *ROOT.glob('apps/client/src/**/*'), *Path(__file__).parent.glob('*'),
                         *ROOT.glob('scripts/windows-regression/*'), ROOT/'scripts/test-android-regression.ps1',ROOT/'scripts/test-windows-regression.ps1'])
        result['sourceHashes'] = {str(p.relative_to(ROOT)).replace('\\','/'): sha(p) for p in source if p.is_file()}
        device = Device(args.adb, args.serial, args.owner_manifest)
        result['device'] = {'serial': args.serial, 'avdName': device.owner['avdName'], 'os': device.text('shell','getprop','ro.build.version.release'), 'api':device.text('shell','getprop','ro.build.version.sdk'), 'abi': device.text('shell','getprop','ro.product.cpu.abi')}
        device.install(apk, replace=True)
        installed = device.text('shell','pm','path',PACKAGE).splitlines()
        require(len(installed)==1 and installed[0].startswith('package:/data/app/'), 'ambiguous installed APK')
        installed_hash = device.text('shell','sha256sum',installed[0].removeprefix('package:')).split()[0]
        require(installed_hash==result['apkSha256'], 'installed APK hash mismatch')
        result['installedApkSha256'] = installed_hash
        result['webviewPackage'] = device.text('shell','dumpsys','webviewupdate').splitlines()[1:4]
        device.clear()
        fixture = Fixture(run)
        device.reverse(int(fixture.origin.rsplit(':',1)[1]))
        device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der', fixture.ca.read_bytes())
        suite = Suite(device, fixture, apk, run)
        suite.launch()
        for stage, function in [('A-CRUD',suite.crud), ('A-TERM',suite.terminal), ('A-PREVIEW',suite.preview),
                                ('A-LIFECYCLE',suite.lifecycle),('A-SAF',suite.saf),('A-UPDATE',suite.update),
                                ('X-TRANSFER',lambda:suite.cross_transfer(args.windows_exe,args.edge_driver))]:
            checks = function()
            result['cases'][stage] = {'passed': True, 'checks': checks}
            print('PASS ' + stage, flush=True)
        result['webview'] = suite.app.metadata
        result['passed'] = all(c['passed'] for c in result['cases'].values())
    except (Exception, KeyboardInterrupt) as error:
        result['failure'] = {'stage':stage, 'type':type(error).__name__}
        if suite and suite.app:
            result['failure']['action'] = suite.app.action
            result['failure']['touchDiagnostic'] = getattr(suite.app, 'touch_diagnostic', None)
            try:
                result['failure']['shellErrorShown'] = bool(suite.app.text('.runtime-sessions [role=alert]'))
                result['failure']['shellInputValid'] = suite.app.evaluate('(()=>{const e=document.querySelector(".runtime-sessions form input");return !e||/^[a-z0-9-]{1,32}$/.test(e.value)})()')
            except Exception:
                pass
        if type(error) is RuntimeError:
            result['failure']['code'] = str(error)
        result['cases'].setdefault(stage, {'passed':False})
    finally:
        timer.cancel()
        for name in ('A-CRUD','A-TERM','A-PREVIEW','A-LIFECYCLE','A-SAF','A-UPDATE','X-TRANSFER'):
            result['cases'].setdefault(name, {'passed':False,'status':'not run after prior failure'})
        if suite and suite.app:
            try:
                suite.app.close()
            except Exception as error:
                result['cleanup']['cdpFailure'] = type(error).__name__
                result['passed'] = False
        if device:
            try:
                result['cleanup']['device'] = device.cleanup()
            except Exception as error:
                result['cleanup']['deviceFailure'] = type(error).__name__
                result['passed'] = False
        if fixture:
            try:
                state = fixture.snapshot()
                result['fixtureSummary'] = {'pngBytes':state['pngBytes'], 'events':state['events'], 'sessions':{n:{k:v for k,v in s.items() if k != 'input'} for n,s in state['sessions'].items()}}
                require(not any(e['method']=='DELETE' and not str(e['name']).startswith('mcshot-') for e in state['events']), 'work Shell deleted')
            except Exception as error:
                result['cleanup']['ledgerFailure'] = type(error).__name__
                result['passed'] = False
            try:
                fixture.close()
                result['cleanup']['fixtureStopped'] = True
            except Exception as error:
                result['cleanup']['fixtureFailure'] = type(error).__name__
                result['passed'] = False
        (run / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps({'passed':result['passed'],'result':str(run / 'result.json')}),flush=True)
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
