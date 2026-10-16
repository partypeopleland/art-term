"""Real APK and isolated official Runtime: burst output and same-token recovery.

Uses only an owned emulator/Runtime. CDP observes actual native IPC; no native
response or terminal write is mocked. The fixed producer models resume output,
not an AI model invocation or a claim about complete conversation history.
"""
import argparse
import asyncio
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import sys
import time
import tomllib
import uuid

from aiohttp import ClientSession, WSMsgType, web
from device import ROOT, require, wait, STORE
from linux_emulator import LinuxEmulator
from ui_matrix import Matrix


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


WireFixture = load('pressure_wire', ROOT / 'scripts/windows-regression/fixture.py').Fixture
Harness = load('pressure_runtime', ROOT / 'scripts/linux-regression/run.py').Harness


class Bridge(WireFixture):
    def __init__(self, root, runtime):
        self.runtime = runtime
        self.attaches, self.admin_calls, self.clients = [], [], {}
        super().__init__(root)
        self.credential = runtime.credential

    async def admin(self, request):
        self.admin_calls.append((request.method, request.path))
        async with ClientSession(trust_env=False) as client:
            async with client.request(request.method,
                    f'http://127.0.0.1:{self.runtime.port}' + request.raw_path,
                    data=await request.read(), headers={
                        'Authorization': request.headers.get('Authorization', ''),
                        'Content-Type': 'application/json'}) as response:
                raw = await response.read()
                if response.status == 200:
                    value = json.loads(raw)
                    if request.method == 'POST' and value.get('session'):
                        self.runtime.owned.add(value['session'])
                    elif request.method == 'DELETE':
                        self.runtime.owned.discard(request.match_info.get('name'))
                return web.Response(body=raw, status=response.status,
                                    content_type='application/json')

    async def pty(self, request):
        name = request.match_info['name']
        self.attaches.append({'name': name, 'since': int(request.query.get('since', 0)),
            'tokenHash': hashlib.sha256(request.headers.get('Authorization', '').encode()).hexdigest()})
        async with ClientSession(trust_env=False) as client:
            async with client.ws_connect(f'http://127.0.0.1:{self.runtime.port}' + request.raw_path,
                    headers={'Authorization': request.headers.get('Authorization', '')},
                    max_msg_size=262144) as upstream:
                downstream = web.WebSocketResponse(max_msg_size=262144)
                await downstream.prepare(request)
                self.clients[name] = downstream

                async def pump(source, target):
                    async for message in source:
                        if message.type == WSMsgType.BINARY:
                            await target.send_bytes(message.data)
                        elif message.type == WSMsgType.TEXT:
                            await target.send_str(message.data)
                        else:
                            break

                tasks = [asyncio.create_task(pump(upstream, downstream)),
                         asyncio.create_task(pump(downstream, upstream))]
                try:
                    await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                finally:
                    for task in tasks:
                        task.cancel()
                    await asyncio.gather(*tasks, return_exceptions=True)
                    if self.clients.get(name) is downstream:
                        self.clients.pop(name, None)
                    await downstream.close(code=upstream.close_code or 1000)
                return downstream

    def overflow(self, name):
        async def close():
            await self.clients[name].close(code=4005)
        self.call(close())

    def close(self):
        async def close_clients():
            await asyncio.gather(*(ws.close() for ws in list(self.clients.values())),
                                 return_exceptions=True)
        self.call(close_clients())
        super().close()


OBSERVE = r'''(()=>{
const send=window.ipc.postMessage.bind(window.ipc), callbacks=window.__TAURI_INTERNALS__.callbacks;
window.__pressure={ended:[],bytes:0,delayNext:false,positions:[]};
for(const [id,callback] of callbacks)if(typeof callback==='function')callbacks.set(id,v=>{
 const e=v?.message??v;
 if(e?.kind==='viewport'||e?.kind==='position'){
  window.__pressure.positions.push(e);
  window.__pressure.positions=window.__pressure.positions.slice(-64);
 }
 return callback(v);
});
window.ipc={postMessage:raw=>{
 const m=JSON.parse(raw),cb=callbacks.get(m.callback);
 if(m.cmd==='mobile_poll'&&cb)callbacks.set(m.callback,v=>{
  const p=window.__pressure;
  if(p.delayNext&&v.runtime?.events?.some(e=>e.kind==='bytes')){
   p.delayNext=false;const end=performance.now()+300;while(performance.now()<end){}
  }
  for(const e of v.runtime?.events||[])if(e.kind==='bytes')p.bytes+=e.bytes.length;
  if(v.runtime?.ended)p.ended.push(v.runtime.ended);
  return cb(v);
 });return send(raw);
}};
})()'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--sdk', type=Path, default=Path('/home/art/Android/Sdk'))
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--diagnostic-without-native', action='store_true',
                        help='Diagnosis only: close the owned native surface, retaining the same APK/stream')
    args = parser.parse_args()
    run = ROOT / 'target/android-regression' / ('connection-' + uuid.uuid4().hex)
    run.mkdir(parents=True, mode=0o700)
    runtime = proxy = device = suite = None
    result = {'apkSha256': hashlib.sha256(args.apk.read_bytes()).hexdigest(), 'cases': {},
              'diagnosticOnly': args.diagnostic_without_native}
    try:
        runtime_root = run / 'runtime'; runtime_root.mkdir()
        runtime = Harness(runtime_root, args.binary)
        proxy_root = run / 'proxy'; proxy_root.mkdir()
        proxy = Bridge(proxy_root, runtime)
        device = LinuxEmulator(args.sdk, run / 'emulator').start_emulator()
        device.install(args.apk)
        device.reverse(int(proxy.origin.rsplit(':', 1)[1]))
        suite = Matrix(device, proxy, args.apk, run, [])
        suite.reset()
        app = suite.app
        app.manage(); suite.add(); suite.close_manage()
        app.close(); device.stop()
        raw = device.store().decode().replace('schema_version = 5', 'schema_version = 6')
        raw += '\n[[desktops]]\nid = "preserved-fixture"\ntoken_ciphertext = [77, 88]\n'
        device.private_write(STORE, raw.encode())
        app = suite.launch()
        require(len(app.profiles()) == 1, 'schema6 upgrade lost existing Host')
        app.manage(); app.click_text('編輯', '[data-host-id] button')
        app.set('name', 'Preserved Host')
        app.click_text('儲存', '[data-testid=runtime-host-form] button')
        wait(lambda: app.profiles()[0]['name'] == 'Preserved Host')
        saved = tomllib.loads(device.store().decode())
        require(saved['schema_version'] == 6 and saved['desktops'][0]['token_ciphertext'] == [77, 88],
                'schema6 desktop settings changed during Host edit')
        suite.close_manage()
        result['cases']['schema6KeystoreUpgrade'] = True
        suite.shell('pressure')
        app.evaluate(OBSERVE)
        if args.diagnostic_without_native:
            reader = wait(lambda: app.evaluate('window.__scrollTest?.readerId'))
            app.evaluate('window.__TAURI_INTERNALS__.invoke("plugin:mobile-history|close",' + json.dumps({'readerId': reader}) + ')')

        def live():
            return app.evaluate('document.querySelector("[data-testid=link-status]")?.classList.contains("live")===true')

        def contains(text):
            return suite.contains('pressure', text)

        def producer(label, delay=0):
            script = f'''import os,time
time.sleep({delay})
for i in range(20000):
 os.write(1, ("\\x1b[32mROW-%05d " % i + "x"*200 + "\\x1b[0m\\n").encode())
with open("once-{label}","a") as f:f.write("once\\n")
print("FINISHED_{label}",flush=True)
'''
            command = 'python3 -c ' + shlex.quote("exec(__import__('base64').b64decode(" + repr(base64.b64encode(script.encode()).decode()) + '))')
            suite.compose(command)

        before = app.evaluate('window.__pressure.bytes')
        app.evaluate('window.__pressure.delayNext=true')
        started = time.monotonic()
        print('START >4MiB live output', flush=True)
        producer('LIVE')
        wait(lambda: contains('FINISHED_LIVE'), 90)
        require(live(), 'live burst disconnected')
        require(not app.evaluate('window.__pressure.ended'), 'normal burst ended the native stream')
        received = app.evaluate('window.__pressure.bytes') - before
        require(received > 4 * 1024 * 1024, 'live burst did not reach real native IPC')
        result['cases']['liveBurst'] = {'bytes': received, 'rendererDelayMs': 300,
                                      'seconds': round(time.monotonic() - started, 2)}
        print('PASS live burst >4MiB, actual APK parsing', flush=True)
        if args.diagnostic_without_native:
            result['passed'] = True
            return

        count = app.evaluate('window.__scrollTest.records.length')
        require(0 < count <= 256, 'initial native page is not bounded')
        time.sleep(.6)
        require(app.evaluate('window.__scrollTest.records.length') == count,
                'idle updates preloaded older pages without user scroll')
        for _ in range(8):
            device.call('shell', 'input', 'swipe', '200', '160', '200', '710', '180')
            time.sleep(.6)
            if app.evaluate('window.__scrollTest.records.length') > count:
                break
        wait(lambda: app.evaluate('window.__scrollTest.records.length') > count, 15)
        require(app.evaluate('window.__scrollTest.records.length') <= 1024, 'native page limit exceeded')
        def preserved_anchor():
            return app.evaluate('''(()=>{
              const events=window.__pressure.positions;
              return events.some((v,i)=>v.kind==='viewport'&&events.slice(i+1).some(p=>
                p.kind==='position'&&p.firstId<v.firstId&&p.restoredAnchorId===v.anchorId&&
                p.restoredAnchorCharacter===v.anchorCharacter&&Math.abs(p.restoredAnchorPixel-v.anchorPixel)<=2));
            })()''')
        wait(preserved_anchor, 15)
        hierarchy = device.text('shell', 'dumpsys', 'activity', 'top')
        (run / 'native-view-hierarchy.txt').write_text(hierarchy)
        # The reader's ScrollView is an anonymous Kotlin subclass, whose
        # actual View hierarchy reports MobileHistoryPlugin$open... rather
        # than the Android base class name. Accessibility can omit this overlay.
        require(('android.widget.ScrollView' in hierarchy or 'MobileHistoryPlugin$open' in hierarchy)
                and 'android.widget.TextView' in hierarchy,
                'actual native Android reader missing')
        result['cases']['nativePaging'] = {'initialRecords': count,
            'loadedRecords': app.evaluate('window.__scrollTest.records.length'),
            'realTouchPrependAnchorPreserved': True}
        print('PASS real Android scroll loads older page and preserves ID/character/pixel anchor', flush=True)

        admin_before = list(proxy.admin_calls)
        attaches_before = len(proxy.attaches)
        original = proxy.attaches[-1]
        proxy.overflow('pressure')
        wait(lambda: len(proxy.attaches) > attaches_before and live(), 30)
        recovered = proxy.attaches[-1]
        require(recovered['tokenHash'] == original['tokenHash'], 'recovery changed retained token')
        require(recovered['since'] > 4 * 1024 * 1024, 'recovery did not use acknowledged cursor')
        require(proxy.admin_calls == admin_before, 'recovery called admin mutation/list')
        suite.compose("printf 'AFTER_RECOVERY\\n'")
        wait(lambda: contains('AFTER_RECOVERY'), 20)
        result['cases']['remote4005Recovery'] = {'sameToken': True, 'since': recovered['since'], 'usable': live()}
        print('PASS injected 4005: same-token ACK cursor recovery and usable input', flush=True)

        # Real output continues after the tab closes, then the same Shell is
        # reopened. Runtime's bounded ring may truthfully report a Gap.
        producer('OFFLINE', 2)
        app.click('.tab.active .close')
        wait(lambda: not app.has('.tab.active'))
        wait(lambda: (runtime.home / 'once-OFFLINE').exists(), 40)
        app.tap('to-list')
        app.tap('runtime-shell-' + app.profiles()[0]['id'])
        wait(lambda: app.has('.runtime-sessions li'))
        app.click_text('連線', '.runtime-sessions li button')
        wait(lambda: live() and contains('FINISHED_OFFLINE'), 60)
        require((runtime.home / 'once-LIVE').read_text() == 'once\n', 'live input replayed')
        require((runtime.home / 'once-OFFLINE').read_text() == 'once\n', 'offline input replayed')
        result['cases']['longReattach'] = {'usable': True, 'inputExecutedOnce': True}
        result['passed'] = True
        device.screenshot(run / 'passed.png')
        print('PASS long reattach, exactly-once producer input', flush=True)
    except Exception as error:
        result.update(passed=False, errorType=type(error).__name__, error=str(error)[:300])
        if suite and suite.app:
            try:
                result['nativeObservation'] = suite.app.evaluate('window.__pressure')
            except Exception:
                pass
        if device and device.verified:
            device.screenshot(run / 'failed.png')
        raise
    finally:
        cleanup_errors = []
        actions = []
        if suite and suite.app:
            actions.append(('cdp', suite.app.close))
        if device and device.verified:
            actions.append(('android', device.cleanup))
        if proxy:
            actions.append(('proxy', proxy.close))
        if runtime:
            actions.append(('runtime', runtime.close))
        if device:
            actions.append(('emulator', device.shutdown))
        for label, close in actions:
            try:
                close()
            except Exception as error:
                cleanup_errors.append(label + ':' + type(error).__name__)
        result['cleanupFinished'] = not cleanup_errors
        if cleanup_errors:
            result.update(passed=False, cleanupErrors=cleanup_errors)
        (run / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
        print(run / 'result.json', flush=True)
        if cleanup_errors:
            raise RuntimeError('owned fixture cleanup failed: ' + ','.join(cleanup_errors))


if __name__ == '__main__':
    main()
