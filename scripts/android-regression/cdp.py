"""Actual Android WebView: native OS taps, CDP text/keys, pass-through observations."""
import json
import re
import time
import urllib.request
import websocket
from device import PACKAGE, require, wait

OBSERVER = r"""(()=>{
if(window.__reg)return;
const i=window.__TAURI_INTERNALS__;if(!i)throw Error('native bridge unavailable');
const r=window.__reg={sessions:{},acks:0,recoveries:0,shots:0,shotStarts:[]};
function record(cmd,a,v){
if(cmd==='mobile_open')r.sessions[v.id]={name:v.remoteSession,tail:'',decoder:new TextDecoder()};
if(cmd==='mobile_poll'){
 const s=r.sessions[a.sessionId];
 function collect(x){if(!x||typeof x!=='object')return;for(const [k,z] of Object.entries(x)){
 if(s&&k==='bytes'&&Array.isArray(z))s.tail=(s.tail+s.decoder.decode(new Uint8Array(z),{stream:true})).slice(-8192);
 else if(z&&typeof z==='object')collect(z)}}collect(v);
}
if(cmd==='gui_runtime_ack')r.acks++;
if(cmd==='gui_runtime_recover')r.recoveries++;
if(cmd==='mobile_screenshot')r.shots++;
}
// Android uses the real Wry Java postMessage interface (Tauri invoke is frozen).
// Wrap only observation callbacks; send the byte-identical original native request.
const native=window.ipc.postMessage.bind(window.ipc);
const observed=data=>{
 const message=JSON.parse(data);const cmd=message.cmd;
 if(cmd.startsWith('mobile_')||cmd==='gui_runtime_ack'||cmd==='gui_runtime_recover'){
  const a={sessionId:message.payload?.sessionId};
  if(cmd==='mobile_screenshot')r.shotStarts.push(performance.now());
  const callback=i.callbacks.get(message.callback);
  if(callback)i.callbacks.set(message.callback,v=>{record(cmd,a,v);return callback(v)});
 }
 return native(data);
};
// Android Java object method assignment is ignored despite its JS descriptor.
// The window property is writable; retain the original bound native method.
window.ipc={postMessage:observed};
if(window.ipc.postMessage!==observed)throw Error('native observation bridge unavailable');
r.mode='native-android-postMessage';
})()"""


class App:
    def __init__(self, device):
        self.device = device
        self.ws = None
        self.sequence = 0
        self.metadata = {}
        self.action = 'connect'

    def connect(self):
        pid = wait(lambda: self.device.text('shell', 'pidof', PACKAGE))
        require(pid.isdigit(), 'ambiguous APK process')
        socket = wait(lambda: next((line.split()[-1].lstrip('@') for line in self.device.text('shell', 'cat', '/proc/net/unix').splitlines()
                                    if 'webview_devtools_remote_' + pid in line), None), 30)
        port = self.device.forward(socket)
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        def targets():
            try:
                with opener.open(f'http://127.0.0.1:{port}/json', timeout=2) as response:
                    return next((t for t in json.load(response) if t.get('type') == 'page'), None)
            except (OSError, ValueError):
                return None
        target = wait(targets)
        self.ws = websocket.create_connection(target['webSocketDebuggerUrl'], timeout=30, suppress_origin=True)
        self.command('Runtime.enable')
        wait(lambda: self.evaluate('!!window.__TAURI_INTERNALS__'))
        self.evaluate(OBSERVER)
        self.metadata = self.command('Browser.getVersion')
        return self

    def command(self, method, params=None):
        self.sequence += 1
        serial = self.sequence
        self.ws.send(json.dumps({'id': serial, 'method': method, 'params': params or {}}))
        while True:
            reply = json.loads(self.ws.recv())
            if reply.get('id') == serial:
                require('error' not in reply, 'CDP operation rejected')
                return reply.get('result', {})

    def evaluate(self, expression):
        reply = self.command('Runtime.evaluate', {'expression': expression, 'returnByValue': True, 'awaitPromise': True})
        require('exceptionDetails' not in reply, 'read-only WebView query failed')
        return reply.get('result', {}).get('value')

    def has(self, css):
        return self.evaluate(f'!!document.querySelector({json.dumps(css)})')

    def text(self, css):
        return self.evaluate(f'document.querySelector({json.dumps(css)})?.textContent || ""')

    def profiles(self):
        return self.evaluate('window.__TAURI_INTERNALS__.invoke("gui_runtime_hosts")')

    def _reported_ime_obstruction(self):
        # API35 can report requested IME visibility with only its navigation
        # strip. Positive WindowManager content height identifies obstruction.
        state = self.device.text('shell', 'dumpsys', 'window', 'displays')
        sources = re.findall(r'(?m)^\s*(?:mSource=)?InsetsSource .*?type=ime frame=\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\].*? visible=(true|false)(?:\s|$)', state)
        heights = re.findall(r'(?m)^\s*mImeHeight=(\d+)\s*$', state)
        require(sources and len(set(sources)) == 1 and heights and len(set(heights)) == 1,
                'unknown or ambiguous native IME inset geometry')
        left, top, right, bottom, visible = sources[0]
        return visible == 'true' and int(right) > int(left) and int(bottom) > int(top) and int(heights[0]) > 0

    def _touch(self, expression, label):
        self.touch_diagnostic = {'phase':'readiness'}
        try:
            return self._touch_impl(expression, label)
        except Exception:
            try:
                self.touch_diagnostic['systemReportedImeObstruction'] = self._reported_ime_obstruction()
                self.touch_diagnostic['dom'] = self.evaluate(f'''(()=>{{const e={expression},a=document.activeElement,v=visualViewport,r=e?.getBoundingClientRect(),x=r?r.x+r.width/2:0,y=r?r.y+r.height/2:0,h=document.elementFromPoint(x,y);return {{exists:!!e,disabled:!!e?.disabled,rect:r?{{x:r.x,y:r.y,width:r.width,height:r.height}}:null,viewport:{{x:v.offsetLeft,y:v.offsetTop,width:v.width,height:v.height}},activeTag:a?.tagName,hitTag:h?.tagName,hitTarget:!!e&&!!h&&(h===e||e.contains(h)),visible:!!r&&x>=v.offsetLeft&&x<v.offsetLeft+v.width&&y>=v.offsetTop&&y<v.offsetTop+v.height}}}})()''')
            except Exception:
                self.touch_diagnostic['snapshotFailed'] = True
            raise

    def _touch_impl(self, expression, label):
        self.action = 'locating touch point ' + label
        wait(lambda:self.evaluate(f'(()=>{{const e={expression};return !!e&&!e.disabled&&e.getBoundingClientRect().width>0}})()'))

        # A touch must own focus changes. Programmatically focusing a button
        # before tapping it would contaminate keyboard visibility assertions.
        self.evaluate(f'({expression}).scrollIntoView({{block:"center",inline:"center"}})')
        self.touch_diagnostic['phase'] = 'native-calibration'
        native_bounds = self.device.webview_bounds()
        viewport = self.evaluate('({x:visualViewport.offsetLeft,y:visualViewport.offsetTop,width:visualViewport.width,height:visualViewport.height,scale:visualViewport.scale})')
        require(viewport['width'] > 0 and viewport['height'] > 0 and viewport['scale'] == 1, 'unsupported native WebView viewport scale')
        sx = (native_bounds[2]-native_bounds[0])/viewport['width']
        sy = (native_bounds[3]-native_bounds[1])/viewport['height']
        clipped_ime = False
        if abs(sx-sy) > max(sx,sy)*.02:
            # Android14 may keep the full WebView accessibility rectangle
            # while Chromium clips only visualViewport for the soft keyboard.
            # CSS coordinates retain the horizontal scale; stretching Y to
            # the full rectangle would tap beneath the intended control.
            shown = re.findall(r'mInputShown=(true|false)', self.device.text('shell','dumpsys','input_method'))
            layout = self.evaluate('({width:innerWidth,height:innerHeight})')
            clipped_ime = (shown and set(shown)=={'true'} and sx < sy
                           and abs(layout['width']*sx-(native_bounds[2]-native_bounds[0])) <= 2
                           and viewport['height']*sx < native_bounds[3]-native_bounds[1])
            require(clipped_ime, 'native WebView viewport aspect mismatch')
            sy = sx
        self.touch_diagnostic['calibration'] = {'bounds':native_bounds,'viewport':viewport,'scaleX':sx,'scaleY':sy}
        self.touch_diagnostic['calibration']['imeClippedViewport'] = bool(clipped_ime)
        self.touch_diagnostic['phase'] = 'stable-geometry'
        previous, stable_since = None, None
        def stable_point():
            nonlocal previous, stable_since
            geometry = self.evaluate(f'''(()=>{{const e={expression};if(!e||e.disabled)return null;
const r=e.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2,h=document.elementFromPoint(x,y),v=visualViewport;
if(!r.width||!r.height)return null;
if(x<v.offsetLeft||x>=v.offsetLeft+v.width||y<v.offsetTop||y>=v.offsetTop+v.height)return {{needsScroll:true}};
if(!h||!(h===e||e.contains(h)))return null;
return {{x,y,w:r.width,h:r.height,vx:v.offsetLeft,vy:v.offsetTop,vw:v.width,vh:v.height,scale:v.scale}}}})()''')
            self.touch_diagnostic['lastGeometry'] = geometry
            now = time.monotonic()
            if geometry and geometry.get('needsScroll'):
                self.evaluate(f'({expression}).scrollIntoView({{block:"center",inline:"center"}})')
                previous, stable_since = None, now
                return None
            if not geometry or geometry != previous:
                previous, stable_since = geometry, now
                return None
            return {'x':geometry['x'],'y':geometry['y']} if now-stable_since >= .4 else None
        point = wait(stable_point)
        current_viewport = self.evaluate('({x:visualViewport.offsetLeft,y:visualViewport.offsetTop,width:visualViewport.width,height:visualViewport.height,scale:visualViewport.scale})')
        require(current_viewport == viewport, 'native WebView viewport changed after calibration')
        physical = {'x':round(native_bounds[0]+(point['x']-viewport['x'])*sx),
                    'y':round(native_bounds[1]+(point['y']-viewport['y'])*sy)}
        require(native_bounds[0] <= physical['x'] < native_bounds[2] and native_bounds[1] <= physical['y'] < native_bounds[3], 'native tap outside WebView bounds')
        self.touch_diagnostic['physicalPoint'] = physical
        # Observe the exact target without cancelling or changing event propagation.
        self.evaluate(f'''(()=>{{const target={expression};const p=window.__reg.touchProbe={{clicked:false}};
p.listener=e=>{{if(e.isTrusted&&(e.target===target||target.contains(e.target)))p.clicked=true}};
document.addEventListener('click',p.listener,true)}})()''')
        self.touch_diagnostic['phase'] = 'dispatch-target-click'
        try:
            self.device.call('shell', 'input', 'tap', str(physical['x']), str(physical['y']))
            try:
                wait(lambda:self.evaluate('window.__reg.touchProbe.clicked'), 3)
            except RuntimeError:
                raise RuntimeError('trusted touch did not produce target click') from None
            self.action = 'touched ' + label
        finally:
            self.evaluate("(()=>{const p=window.__reg.touchProbe;if(p)document.removeEventListener('click',p.listener,true);delete window.__reg.touchProbe})()")

    def click(self, css):
        self._touch(f'document.querySelector({json.dumps(css)})', css)

    def click_text(self, text, css='button'):
        expression = f'[...document.querySelectorAll({json.dumps(css)})].find(e=>e.textContent.trim()==={json.dumps(text)}&&!e.disabled&&e.getBoundingClientRect().width>0)'
        self._touch(expression, 'text ' + text)

    def key(self, key, code, modifiers=0):
        for kind in ('keyDown', 'keyUp'):
            self.command('Input.dispatchKeyEvent', {'type': kind, 'key': key, 'code': code, 'modifiers': modifiers,
                                                   'windowsVirtualKeyCode': {'KeyA':65,'Enter':13,'Backspace':8,'ArrowDown':40,'ArrowUp':38}.get(code,0)})

    def fill(self, css, value):
        self.action = 'input ' + css
        wait(lambda: self.evaluate(f'(()=>{{const e=document.querySelector({json.dumps(css)});return !!e&&!e.disabled}})()'))
        self.evaluate(f'document.querySelector({json.dumps(css)}).focus()')
        self.key('a', 'KeyA', 2)
        self.key('Backspace', 'Backspace')
        self.command('Input.insertText', {'text': value})
        wait(lambda: self.evaluate(f'document.querySelector({json.dumps(css)})?.value==={json.dumps(value)}'))

    def node(self, testid):
        return '[data-testid=' + json.dumps(testid) + ']'

    def tap(self, testid):
        self.click(self.node(testid))

    def set(self, testid, value):
        self.fill(self.node(testid), value)

    def manage(self):
        if self.has('[data-testid=runtime-create]'):
            return
        if self.has('[data-testid=to-list]') and not self.has('[data-testid=runtime-host-manage]'):
            self.tap('to-list')
        self.tap('runtime-host-manage')
        wait(lambda: self.has('[data-testid=runtime-create]'))

    def close(self):
        if self.ws:
            self.ws.close()
            self.ws = None
