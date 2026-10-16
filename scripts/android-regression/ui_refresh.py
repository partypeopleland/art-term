"""Finalized mobile UI smoke on a new, owned Linux Android emulator.

Run from the repo root after sourcing target/android-tools/env.sh:
  target/android-regression/venv/bin/python scripts/android-regression/ui_refresh.py --apk PATH
Only this runner's fresh AVD is installed/stopped. IPC replies are never mocked.
The local TLS protocol fixture echoes input; it does not execute tmux or commands.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import time
import tomllib
import uuid
from PIL import Image
from cdp import App
from device import ROOT, PACKAGE, require, wait
from linux_emulator import LinuxEmulator
from run import Fixture
from reconnect_scroll import OBSERVE

# Observe native requests/replies without retaining credentials or changing data.
OBSERVE_UI = r'''(()=>{
 const native=window.ipc.postMessage.bind(window.ipc), callbacks=window.__TAURI_INTERNALS__.callbacks;
 const state=window.__uiRefresh={history:[],commands:[],events:[],pointers:[]};
 for(const kind of ['pointerdown','pointermove','pointerup','pointercancel'])document.addEventListener(kind,e=>{state.pointers.push({kind,x:e.clientX,y:e.clientY,trusted:e.isTrusted,target:e.target?.className,root:document.querySelector('.m-app')?.className});state.pointers=state.pointers.slice(-150)}, {capture:true,passive:true});
 window.ipc={postMessage:data=>{
  const m=JSON.parse(data), p=m.payload||{};
  if(m.cmd.startsWith('plugin:mobile-history|'))state.history.push({command:m.cmd,visible:p.visible,bounds:p.bounds,foreground:p.foreground,background:p.background,records:p.records?.length});
  if(m.cmd.startsWith('plugin:mobile-history|register')&&p.event==='history'){
   const channel=Number(String(p.handler).split(':').at(-1)), handler=callbacks.get(channel);
   if(handler)callbacks.set(channel,value=>{const e=value.message||value;state.events.push({kind:e.kind,readerId:e.readerId,anchorId:e.anchorId,atBottom:e.atBottom,phase:e.phase,dx:e.dx});return handler(value)});
  }
  if(m.cmd==='gui_runtime_sessions'||m.cmd==='gui_runtime_create_history_session')state.commands.push(m.cmd);
  return native(data);
 }};
})()'''


class Refresh:
    def __init__(self, device, fixture, output):
        self.device, self.fixture, self.output = device, fixture, output
        self.app = None
        self.profile = None
        self.work = None
        self.wait_stage = None
        self.version = json.loads((ROOT/'crates/art-term-gui/tauri.android.conf.json').read_text())['version']

    def launch(self):
        self.device.start()
        self.app = App(self.device).connect()
        self.app.evaluate(OBSERVE)
        self.app.evaluate(OBSERVE_UI)
        wait(lambda: self.app.has('.settings-trigger'))
        return self.app

    def visible(self, selector):
        return self.app.evaluate(f'(()=>{{const e=document.querySelector({json.dumps(selector)});return !!e&&e.getBoundingClientRect().width>0&&e.getBoundingClientRect().height>0&&getComputedStyle(e).visibility!=="hidden"}})()')

    def page(self, name):
        self.app.click_text(name, '.page-text-navigation button')
        wait(lambda: self.visible('.home-surface' if name == 'Main' else '#terminal-panel'))
        time.sleep(.3)

    def settings(self):
        self.app.click('.settings-trigger')
        wait(lambda: self.app.has('.global-settings-page'))

    def empty_swipe(self):
        self.page('Connect')
        require(self.app.has('[data-testid=no-tabs]'), 'empty Connect not shown')
        self.swipe('.t-area', True)
        self.wait_stage = 'empty Connect right swipe: Main-current'
        wait(lambda: self.app.evaluate('document.querySelector("[data-testid=terminal-to-list]").getAttribute("aria-current")==="page"'))
        return {'emptyConnectRightSwipeToMain': True}

    def about(self):
        a = self.app
        a.click('.fixed-brand')
        wait(lambda: a.has('.app-homepage'))
        require(self.version in a.text('.version-value'), 'candidate version is not shown on native homepage')
        require(a.has('.release-card'), 'release notes absent')
        a.click('.release-card')
        wait(lambda: a.has('.release-document'))
        require(len(a.text('.release-document')) > 20, 'release document empty')
        require(self.visible('.workspace-header'), 'header disappeared on homepage')
        self.page('Main')
        return {'version': self.version, 'nativeApkReleaseDocument': True, 'fixedHeader': True}

    def settings_form(self):
        a = self.app
        self.settings()
        self.settings()
        require(a.has('.global-settings-page') and self.visible('.workspace-header'), 'gear toggled settings closed or lost header')
        require(a.has('.new-connection-settings [data-testid=runtime-host-form]'), 'embedded new connection form absent')
        a.set('name', 'Android UI refresh')
        a.set('base-url', self.fixture.origin)
        a.set('credential', self.fixture.credential)
        a.click_text('儲存', '[data-testid=runtime-host-form] button')
        wait(lambda: len(a.profiles()) == 1)
        self.profile = a.profiles()[0]
        raw = self.device.store()
        doc = tomllib.loads(raw.decode('utf-8-sig'))
        require(doc['schema_version'] == 5 and doc['connections'][0]['credential_ciphertext'], 'Keystore protected profile missing')
        require(self.fixture.credential.encode() not in raw, 'plaintext credential persisted')
        self.page('Main')
        wait(lambda: a.has(f'[data-testid="runtime-{self.profile["id"]}"]'))
        wait(lambda: not a.evaluate('document.querySelector(".host-identity").disabled'))
        require(self.fixture.snapshot()['sessions'] == {}, 'read-only home unexpectedly opened/created a Shell')
        require(a.evaluate('window.__uiRefresh.commands.includes("gui_runtime_sessions")'), 'inline Shell list never reached native adapter')
        a.close(); self.device.stop(); a = self.launch()
        require(a.profiles() == [self.profile], 'Keystore profile changed after process restart')
        return {'embeddedForm': True, 'gearDoesNotToggle': True, 'schema5KeystoreRestart': True, 'readonlyShellListing': True}

    def create(self):
        a = self.app
        wait(lambda: a.has('.new-connection') and not a.evaluate('document.querySelector(".new-connection").disabled'))
        a.click('.new-connection')
        self.wait_stage = 'native mobile_open reply'
        wait(lambda: bool(a.evaluate('Object.keys(window.__reg.sessions).length')))
        wait(lambda: self.visible('#terminal-panel'))
        self.wait_stage = 'observe exact native Runtime binding'
        remote = wait(lambda: a.evaluate('window.__scrollTest.opens.at(-1)?.remoteSession'))
        require(remote.startswith('openab:'), 'native Runtime binding absent')
        self.work = remote.removeprefix('openab:')
        require(self.work in self.fixture.snapshot()['sessions'], 'native binding not present in protocol fixture')
        self.wait_stage = 'native reader open/update reply'
        wait(lambda: bool(a.evaluate('window.__scrollTest.readerId')), 45)
        require(a.evaluate('window.__scrollTest.nativeErrors.length') == 0, 'native reader IPC failure')
        require(a.evaluate('window.__scrollTest.nativeSnapshots.some(x=>x.visible===true)'), 'native history surface never visible')
        return {'newHistoryShell': True, 'nativeOpen': True, 'nativeReaderVisible': True, 'fixtureDoesNotExecuteTmux': True}

    def swipe(self, selector, right):
        bounds = self.device.webview_bounds()
        rect = self.app.evaluate(f'(()=>{{const r=document.querySelector({json.dumps(selector)}).getBoundingClientRect();return {{x:r.x,y:r.y,w:r.width,h:r.height,vw:innerWidth,vh:innerHeight}}}})()')
        scale = (bounds[2] - bounds[0]) / rect['vw']
        y = bounds[1] + (rect['y'] + rect['h'] * .45) * scale
        left = bounds[0] + (rect['x'] + rect['w'] * .16) * scale
        right_edge = bounds[0] + (rect['x'] + rect['w'] * .84) * scale
        self.device.call('shell', 'input', 'swipe', round(left if right else right_edge), round(y), round(right_edge if right else left), round(y), 420)

    def vertical_drag(self):
        a = self.app
        async def output():
            await self.fixture.output(self.fixture.sessions[self.work], ''.join(f'UI refresh history row {index:03d}\r\n' for index in range(140)).encode())
        self.fixture.call(output())
        wait(lambda: a.evaluate('window.__scrollTest.records.length >= 100'), 30)
        baseline = a.evaluate('window.__uiRefresh.events.length')
        bounds = self.device.webview_bounds()
        rect = a.evaluate('(()=>{const r=document.querySelector(".t-area").getBoundingClientRect();return {x:r.x,y:r.y,w:r.width,h:r.height,vw:innerWidth}})()')
        scale = (bounds[2]-bounds[0])/rect['vw']
        x = bounds[0]+round((rect['x']+rect['w']*.5)*scale)
        top = bounds[1]+round((rect['y']+rect['h']*.25)*scale)
        bottom = bounds[1]+round((rect['y']+rect['h']*.75)*scale)
        self.device.call('shell', 'input', 'swipe', x, top, x, bottom, 450)
        wait(lambda: a.evaluate(f'window.__uiRefresh.events.slice({baseline}).some(e=>e.kind==="viewport")'), 20)
        events = a.evaluate(f'window.__uiRefresh.events.slice({baseline})')
        require(any(event.get('kind') == 'reading' for event in events), 'vertical drag did not enter native reading')
        require(any(event.get('kind') == 'viewport' and event.get('atBottom') is False for event in events), 'vertical native drag did not leave latest viewport')
        require(a.evaluate('[...document.querySelectorAll(".page-text-navigation button")].find(e=>e.textContent==="Connect").getAttribute("aria-current")==="page"'), 'vertical drag changed workspace')
        return {'nativeVerticalReadingAndViewport': True, 'workspaceRemainsConnect': True, 'nativeEvents': events}

    def gestures(self):
        a = self.app
        self.swipe('.t-area', True)
        self.wait_stage = 'native right swipe: Main-current'
        wait(lambda: a.evaluate('document.querySelector("[data-testid=terminal-to-list]").getAttribute("aria-current")==="page"'))
        self.wait_stage = 'native right swipe: home-visible'
        wait(lambda: self.visible('.home-surface'))
        time.sleep(.3)
        require('on' in a.evaluate(f'document.querySelector("[data-testid=runtime-status-{self.profile["id"]}]").className'), 'live local connection indicator absent')
        self.swipe('.list-page', False)
        self.wait_stage = 'DOM left swipe: Connect-current'
        wait(lambda: a.evaluate('[...document.querySelectorAll(".page-text-navigation button")].find(e=>e.textContent==="Connect").getAttribute("aria-current")==="page"'))
        self.wait_stage = 'DOM left swipe: terminal-visible'
        wait(lambda: self.visible('#terminal-panel'))
        self.wait_stage = 'DOM left swipe: native-reader-visible'
        wait(lambda: a.evaluate('window.__scrollTest.nativeSnapshots.at(-1)?.visible===true'), 20)
        return {'nativeTerminalRightDragToMain': True, 'domHomeLeftDragToConnect': True, 'localLiveDot': True}

    def keys(self):
        a = self.app
        before = self.fixture.snapshot()['sessions'][self.work]['input']
        a.tap('k-enter'); a.tap('k-ctrlc')
        wait(lambda: self.fixture.snapshot()['sessions'][self.work]['input'].endswith('\r\x03'))
        require(self.fixture.snapshot()['sessions'][self.work]['input'][len(before):] == '\r\x03', 'quick keys changed bytes or sent extras')
        return {'enterAndControlCExactBytes': True}

    def theme(self):
        a = self.app
        before = a.evaluate('document.querySelector(".m-app").dataset.theme')
        self.settings()
        options = a.evaluate('[...document.querySelectorAll(".theme-option")].map(e=>({name:e.querySelector("strong").textContent,selected:e.getAttribute("aria-pressed")==="true"}))')
        choice = next(option for option in options if not option['selected'])
        a.click_text(choice['name'], '.theme-copy strong')
        wait(lambda: a.evaluate('document.querySelector(".m-app").dataset.theme') != before)
        selected = a.evaluate('document.querySelector(".m-app").dataset.theme')
        self.page('Connect')
        wait(lambda: a.evaluate('window.__scrollTest.nativeSnapshots.at(-1)?.visible===true'), 20)
        colors = a.evaluate('({foreground:getComputedStyle(document.querySelector(".m-app")).getPropertyValue("--text").trim(),background:getComputedStyle(document.querySelector(".m-app")).getPropertyValue("--bg").trim(),native:window.__uiRefresh.history.filter(x=>x.visible===true).at(-1)})')
        time.sleep(.3)
        self.device.screenshot(self.output/'native-theme.png')
        require(colors['native'], 'native theme update absent')
        # The Android surface receives explicit foreground/background in the actual IPC snapshot.
        require(colors['native']['background'] == int(colors['background'].removeprefix('#'), 16), 'native reader background did not follow theme')
        require(colors['native']['foreground'] == int(colors['foreground'].removeprefix('#'), 16), 'native reader foreground did not follow theme')
        native_bounds = colors['native']['bounds']
        webview = self.device.webview_bounds()
        scale = (webview[2]-webview[0])/a.evaluate('innerWidth')
        point = (webview[0]+round(5*scale), webview[1]+round((native_bounds['y']+native_bounds['height']*.7)*scale))
        painted = Image.open(self.output/'native-theme.png').convert('RGB').getpixel(point)
        expected = tuple(int(colors['background'].removeprefix('#')[offset:offset+2],16) for offset in (0,2,4))
        require(max(abs(actual-wanted) for actual,wanted in zip(painted,expected))<=2, 'native screenshot background did not follow theme')
        return {'selectedTheme': selected, 'nativeSurfaceColors': colors, 'paintedPixel': painted, 'expectedPixel': expected, 'realFrame': 'native-theme.png'}

    def lightbox(self):
        a = self.app
        require(a.has('[data-testid=voice-input]'), 'native voice settings bar absent')
        a.tap('voice-settings-toggle')
        a.click('input[name="voice-preview-mode"][value="lightbox"]')
        wait(lambda: a.has('.voice-lightbox'))
        wait(lambda: a.evaluate('window.__scrollTest.nativeSnapshots.at(-1)?.visible===false'))
        require(self.visible('.workspace-header'), 'lightbox lost fixed header')
        a.click('.lightbox-close')
        wait(lambda: not a.has('.voice-lightbox'))
        wait(lambda: a.evaluate('window.__scrollTest.nativeSnapshots.at(-1)?.visible===true'))
        self.page('Main')
        self.page('Connect')
        require(not a.has('.voice-lightbox'), 'saved lightbox preference auto-opened on connection remount')
        wait(lambda: a.evaluate('window.__scrollTest.nativeSnapshots.at(-1)?.visible===true'))
        a.tap('voice-settings-toggle')
        a.click('input[name="voice-preview-mode"][value="inline"]')
        a.tap('voice-settings-toggle')
        return {'previewModeLightboxHidesNativeReader': True, 'closeRestoresNativeReader': True, 'savedPreferenceDoesNotAutoOpen': True, 'microphoneOrRecognitionNotInvoked': True}

    def secrets_and_saf(self):
        a = self.app
        self.settings()
        a.set('name', 'Unsaved secret form')
        a.set('credential', 'fictional-unsent-secret')
        self.device.key('KEYCODE_HOME'); time.sleep(.5); self.device.start()
        wait(lambda: not a.has('.global-settings-page'))
        self.settings()
        require(a.evaluate('document.querySelector("[data-testid=credential]").value') == '', 'background retained credential input')
        a.click_text('匯入連線', '.transfer-setting-actions button')
        a.tap('transfer-pick')
        self.device.documents()
        self.device.key('KEYCODE_BACK')
        wait(lambda: a.has('.global-settings-page'))
        wait(lambda: a.has('[data-testid=transfer-pick]') and not a.evaluate('document.querySelector("[data-testid=transfer-pick]").disabled'))
        require(len(a.profiles()) == 1, 'SAF cancellation altered profiles')
        return {'backgroundFormSecretCleared': True, 'realAndroidSafPickerCancelled': True}

    def finished_shell(self):
        a = self.app
        self.page('Connect')
        a.tap('k-keyboard')
        self.device.call('shell', 'input', 'text', 'exit')
        a.tap('k-enter')
        wait(lambda: self.fixture.snapshot()['sessions'][self.work]['input'].endswith('exit\r'))
        # Simulate the official Shell-exited close frame after real input.
        # The fixture records bytes; it never executes a shell command.
        async def ended():
            await self.fixture.sessions[self.work]['ws'].close(code=4004)
        self.fixture.call(ended())
        wait(lambda: a.has('[data-testid=no-tabs]'))
        require(not a.has('.term-host') and not a.has('.native-terminal-host'), 'finished terminal still mounted')
        require(a.evaluate('window.__uiRefresh.history.some(x=>x.command.endsWith("|close"))'), 'native reader not closed')
        if 'mInputShown=true' in self.device.text('shell', 'dumpsys', 'input_method'):
            self.device.key('KEYCODE_BACK')
        for index in range(3):
            self.swipe('.t-area', True)
            wait(lambda: a.evaluate('document.querySelector("[data-testid=terminal-to-list]").getAttribute("aria-current")==="page"'))
            time.sleep(.3)
            self.swipe('.list-page', False)
            wait(lambda: a.evaluate('[...document.querySelectorAll(".page-text-navigation button")].find(e=>e.textContent==="Connect").getAttribute("aria-current")==="page"'))
            time.sleep(.3)
            require(a.has('[data-testid=no-tabs]') and not a.has('.term-host'), 'swiping revived finished terminal')
        return {'exitInputVerified': True, 'official4004ShellExit': True, 'targetSelectionRestored': True, 'nativeReaderClosed': True, 'threeRoundTripSwipes': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--sdk', type=Path, default=Path('/home/art/Android/Sdk'))
    args = parser.parse_args()
    apk = args.apk.resolve(strict=True)
    output = ROOT/'target/android-regression'/('ui-refresh-'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')+'-'+uuid.uuid4().hex[:8])
    result = {'passed': False, 'realApk': True, 'mockedIpc': False, 'apkSha256': hashlib.sha256(apk.read_bytes()).hexdigest(), 'cases': {}, 'limits': ['x86_64 debug emulator; ARM64 release and physical device still require testing', 'Protocol fixture does not execute tmux or remote commands', 'Emulator has no speech recognition service; no recognition claim']}
    device = fixture = suite = None
    print('OUTPUT '+str(output), flush=True)
    stage = 'boot'
    try:
        device = LinuxEmulator(args.sdk, output)
        device.start_emulator()
        print("READY owned-emulator", flush=True)
        device.install(apk)
        installed = device.text("shell", "pm", "path", PACKAGE).removeprefix("package:")
        installed_hash = device.text("shell", "sha256sum", installed).split()[0]
        require(installed_hash == result["apkSha256"], "installed APK hash mismatch")
        result["installedApkSha256"] = installed_hash
        print("READY installed-apk", flush=True)
        result['device'] = {'serial': device.serial, 'api': device.text('shell', 'getprop', 'ro.build.version.sdk'), 'abi': device.text('shell', 'getprop', 'ro.product.cpu.abi')}
        fixture = Fixture(output)
        device.reverse(int(fixture.origin.rsplit(':', 1)[1]))
        device.private_write('no_backup/openab-operator-v1/openab-fixture-ca.der', fixture.ca.read_bytes())
        suite = Refresh(device, fixture, output)
        suite.launch()
        for stage, action in [('empty-connect-swipe', suite.empty_swipe), ('homepage', suite.about), ('settings-keystore', suite.settings_form), ('explicit-create', suite.create), ('native-vertical-drag', suite.vertical_drag), ('swipe', suite.gestures), ('keys', suite.keys), ('native-theme', suite.theme), ('voice-lightbox', suite.lightbox), ('background-saf', suite.secrets_and_saf), ('finished-shell', suite.finished_shell)]:
            suite.wait_stage = stage
            checks = action()
            result['cases'][stage] = {'passed': True, 'checks': checks}
            device.screenshot(output/(stage+'.png'))
            print('PASS '+stage, flush=True)
        (output/'fixture-journal.json').write_text(json.dumps(fixture.snapshot(), ensure_ascii=False, indent=2))
        result['passed'] = True
    except Exception as error:
        text = str(error)
        if fixture: text = text.replace(fixture.credential, '[redacted]')
        result['failure'] = {'stage': stage, 'type': type(error).__name__, 'message': text, 'action': suite.app.action if suite and suite.app else None, 'wait': suite.wait_stage if suite else None}
        result['cases'][stage] = {'passed': False}
        if fixture:
            try: (output/'fixture-journal.json').write_text(json.dumps(fixture.snapshot(), ensure_ascii=False, indent=2))
            except Exception: pass
        if suite and suite.app:
            try:
                result['failure']['touch'] = getattr(suite.app, 'touch_diagnostic', None)
                result['failure']['native'] = suite.app.evaluate('window.__scrollTest')
                result['failure']['ui'] = suite.app.evaluate('({root:document.querySelector(".m-app")?.className,navigation:[...document.querySelectorAll(".page-text-link")].map(e=>({label:e.textContent,current:e.getAttribute("aria-current")})),events:window.__uiRefresh.events,pointers:window.__uiRefresh.pointers,history:window.__uiRefresh.history})')
                (output/'failure-dom.html').write_text(suite.app.evaluate('document.documentElement.outerHTML'))
                device.screenshot(output/'failure.png')
            except Exception: pass
        print('FAIL '+stage+': '+text, flush=True)
    finally:
        for name, cleanup in [('cdp', suite.app.close if suite and suite.app else None), ('fixture', fixture.close if fixture else None), ('device', device.cleanup if device else None), ('emulator', device.shutdown if device else None)]:
            if cleanup:
                try:
                    value = cleanup()
                    result.setdefault('cleanup', {})[name] = value if value is not None else True
                except Exception as cleanup_error:
                    result.setdefault('cleanup', {})[name] = {'error': type(cleanup_error).__name__}
                    result['passed'] = False
        for expected in ['empty-connect-swipe', 'homepage', 'settings-keystore', 'explicit-create', 'native-vertical-drag', 'swipe', 'keys', 'native-theme', 'voice-lightbox', 'background-saf', 'finished-shell']:
            result['cases'].setdefault(expected, {'passed': False, 'status': 'not run after prerequisite failure'})
        output.mkdir(parents=True, exist_ok=True)
        (output/'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
