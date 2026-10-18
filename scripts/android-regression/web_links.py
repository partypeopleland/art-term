#!/usr/bin/env python3
"""Verify real native URL intents and reader taps on a fresh owned Android AVD."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import uuid

from cdp import App
from device import PACKAGE, ROOT, require, wait
from linux_emulator import LinuxEmulator


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', required=True, type=Path)
    parser.add_argument('--apk', required=True, type=Path)
    args = parser.parse_args()
    badging = subprocess.check_output([str(args.sdk/'build-tools/35.0.0/aapt'), 'dump', 'badging', str(args.apk)], text=True)
    require('application-debuggable' in badging and "native-code: 'x86_64'" in badging, 'requires own debug x86_64 APK')
    run = ROOT/'target/android-regression'/('run-web-links-'+uuid.uuid4().hex[:12])
    device = LinuxEmulator(args.sdk, run)
    app = None
    result = {'passed': False, 'actualAPK': True, 'mockIPC': False,
              'apkSha256': hashlib.sha256(args.apk.read_bytes()).hexdigest(), 'cases': {}, 'cleanup': {}}
    print('RESULT_DIR '+str(run), flush=True)

    def foreground():
        raw = device.text('shell', 'dumpsys', 'activity', 'activities')
        match = re.search(r'(?:topResumedActivity|mResumedActivity)[:=].*? u\d+ ([\w.]+)/', raw)
        return match.group(1) if match else None

    def browser_opened(url, label):
        package = wait(lambda: (p if p and p != PACKAGE and p != 'android' else None)
                       if (p := foreground()) else None, 15)
        activities = device.text('shell', 'dumpsys', 'activity', 'activities')
        authority = '/'.join(url.split('/')[:3]).replace('HTTPS:', 'https:').replace('HTTP:', 'http:')
        require('android.intent.action.VIEW' in activities and authority in activities, 'actual browser Intent scheme/host mismatch')
        result['cases'][label] = {'browserPackage': package, 'requestedUrl': url, 'observedIntentAuthority': authority}
        device.screenshot(run/(label+'.png'))
        device.start()
        wait(lambda: foreground() == PACKAGE)

    def projection(url):
        snapshot = {'readerId': 'web-links-test', 'visible': True, 'followLatest': False,
                    'records': [{'id': 1, 'text': '測試 '+url, 'wrapped': False, 'styles': []}],
                    'bounds': {'x': 10, 'y': 280, 'width': 390, 'height': 160, 'viewportWidth': 412}}
        app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|open",'+json.dumps(snapshot)+')')
        node = device.native_node(lambda n: url in n.get('text', '') and n.get('class') == 'android.widget.TextView')
        bounds = list(map(int, re.findall(r'\d+', node.attrib['bounds'])))
        # The short URL follows two CJK cells and a space; x=90 lies inside it.
        return bounds[0]+90, bounds[1]+14

    try:
        device.start_emulator()
        device.install(args.apk)
        device.start()
        app = App(device).connect()
        app.evaluate('window.__webLinkWrites=0; const original=window.ipc.postMessage.bind(window.ipc); window.ipc={postMessage:data=>{const m=JSON.parse(data);if(m.cmd==="mobile_write")window.__webLinkWrites++;return original(data)}}')
        for url in ['javascript:alert(1)', 'file:///tmp/test', 'https:///','https://example.com/\nunsafe']:
            reply = app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|openUrl",{url:'+json.dumps(url)+'}).then(()=>false,()=>true)')
            current = foreground()
            result.setdefault('invalidUrlObservations', []).append({'url': url, 'rejected': reply, 'foreground': current})
            require(reply is True and current == PACKAGE, 'non-web URL rejection or foreground check failed')
        result['cases']['invalidUrlsRejected'] = True
        url = 'HTTPS://127.0.0.1:9/ipc-link'
        app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|openUrl",{url:'+json.dumps(url)+'})')
        browser_opened(url, 'nativeIpcHttps')
        for scheme in ['http', 'https']:
            url = scheme+'://127.0.0.1:9/reader-link'
            x, y = projection(url)
            device.call('shell', 'input', 'tap', x, y)
            browser_opened(url, 'nativeReaderTap'+scheme.title())
            app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|close",{readerId:"web-links-test"})')
        url = 'https://127.0.0.1:9/gesture-link'
        x, y = projection(url)
        device.call('shell', 'input', 'swipe', x, y, x, y+55, 250)
        require(foreground() == PACKAGE, 'vertical gesture opened browser')
        app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|close",{readerId:"web-links-test"})')
        x, y = projection(url)
        device.call('shell', 'input', 'swipe', x, y, x, y, 850)
        require(foreground() == PACKAGE, 'long press opened browser')
        require(app.evaluate('window.__webLinkWrites') == 0, 'URL action leaked terminal input')
        result['cases']['gesturesStayInAppAndNoPtyInput'] = True
        result['passed'] = True
    except Exception as error:
        result['failure'] = {'type': type(error).__name__, 'message': str(error)[:300]}
        try:
            (run/'activities-failure.txt').write_text(device.text('shell', 'dumpsys', 'activity', 'activities'))
            device.screenshot(run/'failure.png')
        except Exception: pass
    finally:
        if app:
            try: app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|close",{readerId:"web-links-test"})')
            except Exception: pass
            app.close()
        try:
            if device.process and device.process.poll() is None:
                device.cleanup()
            result['cleanup']['adb'] = True
        except Exception as error: result['cleanup']['adbFailure'] = type(error).__name__
        try:
            device.shutdown()
            result['cleanup']['emulatorStopped'] = True
        except Exception as error: result['cleanup']['emulatorFailure'] = type(error).__name__
        (run/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2), flush=True)
    return 0 if result['passed'] and all(result['cleanup'].get(k) for k in ['adb', 'emulatorStopped']) else 1


if __name__ == '__main__':
    raise SystemExit(main())
