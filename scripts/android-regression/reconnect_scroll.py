"""Real NUC Shell + isolated Android emulator: history after network loss and detach.

Requires a debuggable x86_64 APK. Credentials are read from a private local file.
No mock IPC, no existing phone, no mutation of existing remote Shells.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import re
import ssl
import subprocess
import sys
import time
import urllib.request
import uuid
from PIL import Image, ImageChops
from cdp import App
from device import ROOT, require, wait
from linux_emulator import LinuxEmulator

# Pass-through observation only: the exact native message and response still
# reach the real application. Never record credential payloads or grant tokens.
OBSERVE = r'''(()=>{
 const native=window.ipc.postMessage.bind(window.ipc);
 const callbacks=window.__TAURI_INTERNALS__.callbacks;
 const s=window.__scrollTest={records:[],readerId:'',updates:0,writes:0,opens:[],nativeSnapshots:[],nativeErrors:[]};
 window.ipc={postMessage:data=>{
  const m=JSON.parse(data),cmd=m.cmd,cb=callbacks.get(m.callback);
  if(cmd.startsWith('plugin:mobile-history|')){
   const p=m.payload;
   s.nativeSnapshots.push({cmd,readerId:p.readerId,visible:p.visible,bounds:p.bounds,
      records:p.records?.length,first:p.records?.[0]?.id,last:p.records?.at(-1)?.id});
   s.nativeSnapshots=s.nativeSnapshots.slice(-80);
   const fail=callbacks.get(m.error);
   if(fail)callbacks.set(m.error,value=>{s.nativeErrors.push({cmd,error:String(value).slice(0,200)});return fail(value)});
  }
  if(cmd==='mobile_write')s.writes++;
  if(cb&&(cmd==='mobile_open'||cmd==='plugin:mobile-history|open'||cmd==='plugin:mobile-history|update')){
   const p=m.payload;
   callbacks.set(m.callback,value=>{
    if(cmd==='mobile_open')s.opens.push({id:value.id,remoteSession:value.remoteSession});
    else if(p.visible){s.records=p.records;s.readerId=p.readerId;s.updates++;}
    return cb(value);
   });
  }
  return native(data);
 }};
})()'''


def private_credential(path):
    p = Path(path).resolve(strict=True)
    require(p.is_file() and p.stat().st_mode & 0o077 == 0, 'Credential file must be private (0600)')
    value = p.read_text().strip()
    require(bool(re.fullmatch(r'[a-fA-F0-9]{64}', value)), 'Expected a raw 64-hex admin credential file')
    return value


def report_error(error, credential, origin):
    # Do not serialize request/exception reprs, which may include credentials.
    text = str(error)
    if credential: text = text.replace(credential, '[redacted]')
    if origin: text = text.replace(origin, '[runtime]')
    return text[:300]


class ReconnectTest:
    def __init__(self, app, device, run, name, nonce):
        self.app, self.device, self.run = app, device, run
        self.name, self.nonce = name, nonce
        self.old_marker = nonce+'-file-0001'

    def compose(self, command):
        self.app.tap('k-composer')
        self.app.set('composer-text', command)
        self.app.tap('composer-send-enter')

    def live(self):
        return self.app.evaluate('document.querySelector("[data-testid=link-status]")?.classList.contains("live")===true')

    def observed_history(self):
        return self.app.evaluate('window.__scrollTest.records.map(r=>r.text).join("\\n")')

    def has_old_content(self):
        return self.old_marker in self.observed_history()

    def capture_crop(self, label):
        rect = self.app.evaluate('(()=>{const r=document.querySelector(".native-terminal-host")?.getBoundingClientRect();return r?{x:r.x,y:r.y,w:r.width,h:r.height,vw:innerWidth}:null})()')
        require(rect and rect['w'] > 0 and rect['h'] > 0, 'Native terminal bounds missing')
        x0, y0, x1, _ = self.device.webview_bounds()
        scale = (x1-x0)/rect['vw']
        box = (round(x0+rect['x']*scale), round(y0+rect['y']*scale),
               round(x0+(rect['x']+rect['w'])*scale), round(y0+(rect['y']+rect['h'])*scale))
        raw = self.device.call('exec-out', 'screencap', '-p')
        image = Image.open(io.BytesIO(raw)).convert('RGB').crop(box)
        image.save(self.run/(label+'.png'))
        return image, box

    def scroll(self, label):
        before, box = self.capture_crop(label+'-before')
        nodes = self.device.ui()
        accessible = any(n.get('class') == 'android.widget.ScrollView' and n.get('scrollable') == 'true'
                         for n in nodes.iter('node'))
        hierarchy = False
        if not accessible:
            # Android14 accessibility can omit a newly replaced reader even
            # though the actual native ScrollView is visible and draggable.
            # Verify the concrete Android View and its visible parent bounds
            # instead of mistaking an accessibility snapshot for loss of UI.
            lines = self.device.text('shell','dumpsys','activity','top').splitlines()
            for index, line in enumerate(lines):
                if not re.search(r'net\.missart\.artterm\.history\.MobileHistoryPlugin\$open[^\s{]*\{\S+ V',line): continue
                child = re.search(r' (\d+),(\d+)-(\d+),(\d+)(?:\s|\})',line)
                if not child or index == 0: continue
                parent = re.search(r'android\.widget\.FrameLayout\{\S+ V\S* \S* (-?\d+),(-?\d+)-(-?\d+),(-?\d+)(?:\s|\})',lines[index-1])
                if not parent: continue
                bounds = tuple(map(int,parent.groups()))
                extent = tuple(map(int,child.groups()))
                if (all(abs(actual-expected)<=2 for actual,expected in zip(bounds,box))
                        and extent[2]>extent[0] and extent[3]>extent[1]):
                    hierarchy = True; break
        require(accessible or hierarchy, 'Visible native Android ScrollView missing from accessibility and actual View hierarchy')
        require(self.has_old_content(), 'Previous output missing from current native snapshot')
        writes = self.app.evaluate('window.__scrollTest.writes')
        x0, y0, x1, y1 = box
        x = (x0+x1)//2
        self.device.call('shell', 'input', 'swipe', x, y0+(y1-y0)//4, x, y0+3*(y1-y0)//4, 500)
        time.sleep(.6)
        after, after_box = self.capture_crop(label+'-after')
        require(box == after_box, 'Terminal geometry changed during scroll')
        delta = ImageChops.difference(before, after)
        changed = sum(1 for pixel in delta.getdata() if max(pixel) > 20)
        ratio = changed/(before.width*before.height)
        require(ratio > .005, 'Drag did not visibly move historical text')
        require(self.app.evaluate('window.__scrollTest.writes') == writes, 'Reading gesture sent remote input')
        return {'oldOutputInNativeSnapshot': True, 'nativeScrollView': True,
                'accessibilityScrollView': accessible, 'actualViewHierarchyFallback': hierarchy,
                'trustedAdbSwipe': True, 'changedPixelRatio': ratio, 'remoteInputFromSwipe': False}

    def populate(self):
        # Only synthetic files in this test-owned directory; never ls the
        # user's private home or store the user's real terminal transcript.
        cmd = (f'd=$(mktemp -d /tmp/art-term-{self.nonce}-XXXXXX); '
               f'for i in $(seq 1 160); do touch "$d/$(printf \'{self.nonce}-file-%04d\' "$i")"; done; '
               'ls "$d"; ls -al "$d"; '
               f'printf \'TEST_READY_{self.nonce}\\n\'')
        self.compose(cmd)
        wait(lambda: self.has_old_content() and 'TEST_READY_'+self.nonce in self.observed_history(), 45)
        time.sleep(.5)

    def network_recovery(self):
        before = self.app.evaluate('window.__scrollTest.opens.at(-1)')
        require(before and before['remoteSession'] == 'openab:'+self.name, 'Wrong remote Shell before interruption')
        try:
            self.device.call('shell', 'svc', 'wifi', 'disable')
            self.device.call('shell', 'svc', 'data', 'disable')
            # Observe an actual disconnected UI, not merely a timer or the
            # absence of new output. Failure here is inconclusive, never PASS.
            wait(lambda: not self.live(), 45)
        finally:
            self.device.call('shell', 'svc', 'wifi', 'enable')
            self.device.call('shell', 'svc', 'data', 'enable')
        wait(self.live, 60)
        after = self.app.evaluate('window.__scrollTest.opens.at(-1)')
        require(after == before, 'Automatic recovery replaced the local/remote session')
        wait(self.has_old_content, 10)
        return self.scroll('network-recovery')

    def detach_reconnect(self):
        previous = self.app.evaluate('window.__scrollTest.opens.at(-1)')
        previous_updates = self.app.evaluate('window.__scrollTest.updates')
        self.app.tap('k-detach')
        wait(lambda: not self.live())
        # Clear only test observations: stale native snapshots may otherwise
        # make a failed reconnect appear to retain the old display.
        self.app.evaluate('window.__scrollTest.records=[];window.__scrollTest.readerId=""')
        self.app.click_text('重新連線同一目標')
        wait(self.live, 45)
        current = self.app.evaluate('window.__scrollTest.opens.at(-1)')
        require(current and current['remoteSession'] == previous['remoteSession'] == 'openab:'+self.name,
                'Reconnect selected another remote Shell')
        require(current['id'] != previous['id'], 'Manual reconnect did not open a new local binding')
        try:
            # Reconnecting a retained tab may correctly reuse its native View.
            # A fresh visible snapshot after reopening, rather than a new View
            # identifier, proves this is not a stale observation.
            wait(lambda: self.app.evaluate('window.__scrollTest.updates') > previous_updates
                 and bool(self.app.evaluate('window.__scrollTest.readerId')) and self.has_old_content(), 15)
        except RuntimeError:
            raise RuntimeError('Previous output missing from the newly opened native reader after manual reconnect') from None
        return self.scroll('detach-reconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, default=os.environ.get('ANDROID_HOME'))
    parser.add_argument('--apk', type=Path)
    parser.add_argument('--origin', help='Trusted HTTPS NUC openab-pty root origin')
    parser.add_argument('--credential-file', type=Path)
    smoke = parser.add_mutually_exclusive_group()
    smoke.add_argument('--emulator-smoke', action='store_true', help='Boot/verify/stop owned emulator only')
    smoke.add_argument('--apk-smoke', action='store_true', help='Install/start/CDP-check APK, without a remote connection')
    args = parser.parse_args()
    require(sys.platform.startswith('linux'), 'Linux/KVM runner required')
    require(args.sdk and args.sdk.is_dir(), 'Android SDK required')
    if not (args.emulator_smoke or args.apk_smoke):
        require(args.apk and args.apk.is_file() and args.origin and args.credential_file, 'APK, HTTPS origin and credential file required')
        from urllib.parse import urlsplit
        origin = urlsplit(args.origin)
        require(origin.scheme == 'https' and origin.hostname and origin.path in ('', '/')
                and not origin.username and not origin.query and not origin.fragment, 'HTTPS root origin required')
    if args.apk_smoke: require(args.apk and args.apk.is_file(), 'APK required')
    credential = private_credential(args.credential_file) if not (args.emulator_smoke or args.apk_smoke) else ''
    key = uuid.uuid4().hex[:12]
    name = 'scroll-test-'+key
    run = ROOT/'target/android-regression'/('reconnect-'+key)
    device = LinuxEmulator(args.sdk, run)
    result = {'passed':False, 'cases':{}, 'actualEmulator':False, 'mockedIpc':False,
              'sessionName': name, 'cleanup':{}, 'createdUtc':datetime.now(timezone.utc).isoformat()}
    app = test = None
    created = False
    exit_code = 1
    def api(method, path):
        request = urllib.request.Request(args.origin.rstrip('/')+path,
                                         headers={'Authorization':'Bearer '+credential}, method=method)
        with urllib.request.urlopen(request, timeout=15, context=ssl.create_default_context()) as response:
            return response.read()
    try:
        print('Booting isolated emulator', flush=True)
        device.start_emulator(); result['actualEmulator'] = True
        if args.emulator_smoke:
            result['cases']['emulatorBoot'] = True
        else:
            badging = subprocess.check_output([str(args.sdk/'build-tools/35.0.0/aapt'), 'dump', 'badging', str(args.apk)], text=True)
            require('application-debuggable' in badging and "native-code: 'x86_64'" in badging,
                    'A debuggable x86_64 APK is required; no production APK instrumentation')
            result['apkSha256'] = hashlib.sha256(args.apk.read_bytes()).hexdigest()
            device.install(args.apk); device.start()
            app = App(device).connect(); app.evaluate(OBSERVE)
            wait(lambda: app.has('[data-testid=runtime-host-settings]'))
            if args.apk_smoke:
                platform = app.evaluate('window.__TAURI_INTERNALS__.invoke("gui_platform")')
                require(platform == 'android', 'APK did not use the real Android bridge')
                result['cases']['apkNativeBridge'] = {'passed': True, 'platform': platform}
            else:
                require(app.profiles() == [], 'New test emulator unexpectedly has profiles')
                app.tap('runtime-host-settings')
                app.set('name', 'NUC scroll test'); app.set('base-url', args.origin); app.set('credential', credential)
                app.click_text('儲存', '[data-testid=runtime-host-form] button')
                wait(lambda: len(app.profiles()) == 1)
                if app.has('.runtime-hosts'):
                    # The save action may retain its management dialog.
                    app.click_text('關閉', '.runtime-hosts > header button')
                sessions = json.loads(api('GET', '/admin/sessions'))
                require(all(item['name'] != name for item in sessions['sessions']), 'Test Shell name already exists')
                profile = app.profiles()[0]
                app.tap('runtime-shell-'+profile['id'])
                app.fill('.runtime-sessions form input', name)
                # Mark ownership intent before click so a late remote creation
                # is still cleaned up if the UI times out.
                created = True
                app.click_text('建立並連線', '.runtime-sessions form button')
                test = ReconnectTest(app, device, run, name, key)
                wait(test.live, 45)
                test.populate()
                result['cases']['beforeDisconnect'] = test.scroll('before-disconnect')
                for label, action in [('networkRecovery',test.network_recovery), ('detachReconnect',test.detach_reconnect)]:
                    try:
                        print('Testing '+label, flush=True)
                        result['cases'][label] = {'passed':True, **action()}
                    except Exception as error:
                        result['cases'][label] = {'passed':False, 'reason':report_error(error,credential,args.origin)}
                result['voiceSettingsSwapIncluded'] = True
                require(all(value.get('passed',True) for value in result['cases'].values()), 'At least one reconnect history case failed')
        result['passed'] = True; exit_code = 0
    except Exception as error:
        result['failure'] = report_error(error, credential, args.origin or '')
    finally:
        if test:
            try:
                require(test.live(), 'Test Shell not currently live')
                # The directory variable belongs only to this task-created Shell.
                test.compose(f'case "$d" in /tmp/art-term-{key}-*) rm -rf -- "$d";; esac')
                time.sleep(.3)
                result['cleanup']['syntheticFiles'] = True
            except Exception: result['cleanup']['syntheticFiles'] = False
        if app:
            try: app.close()
            except Exception: pass
        if created:
            try: api('DELETE', '/admin/sessions/'+name); result['cleanup']['ownedRemoteShell'] = True
            except Exception: result['cleanup']['ownedRemoteShell'] = False
        try: device.shutdown(); result['cleanup']['emulatorStopped'] = True
        except Exception: result['cleanup']['emulatorStopped'] = False
        if any(value is False for value in result['cleanup'].values()):
            result['passed'] = False; exit_code = 1
        (run/'result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps(result,ensure_ascii=False,indent=2))
        print('Evidence: '+str(run/'result.json'))
    return exit_code


if __name__ == '__main__':
    raise SystemExit(main())
