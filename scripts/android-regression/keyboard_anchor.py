"""Owned actual APK/Runtime: opening IME must preserve a mid-page anchor.

The sampling update changes only an offscreen line's paint, forcing existing
native position telemetry. It does not mock native geometry or scroll responses.
"""
import argparse, hashlib, json, re, shlex, shutil, tempfile, time, uuid
from pathlib import Path
from connection_pressure import ROOT, Harness, Bridge, Matrix, LinuxEmulator, OBSERVE
from device import require, wait, runtime_work_name

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--apk', type=Path, required=True)
p.add_argument('--history', action='store_true')
args = p.parse_args()
run = ROOT / 'target/android-regression' / ('keyboard-' + uuid.uuid4().hex)
run.mkdir(mode=0o700)
h = f = d = s = None
short = tempfile.TemporaryDirectory(prefix="kh-") if args.history else None
result = {'passed': False, 'apkSha256': hashlib.sha256(args.apk.read_bytes()).hexdigest()}
try:
    r = Path(short.name) if short else run / 'runtime'
    r.mkdir(exist_ok=True); h = Harness(r, Path('/home/art/.local/share/my-cli/openab-pty/openab-pty'))
    if args.history:
        support=h.helper/'tmux-v2';support.mkdir()
        for source,name in [(ROOT/'scripts/openab-runtime/tmux-v2/tmux-shell.py','tmux-shell.py'),(ROOT/'target/tmux-history/tmux-v2/tmux-history.cjs','tmux-history.cjs'),(ROOT/'target/tmux-lazy-proof/tmux-3.6a/tmux','tmux')]:
            shutil.copyfile(source,support/name);(support/name).chmod(0o700)
        (h.helper/'node').symlink_to(ROOT/'target/android-tools/node-v22.23.3-linux-x64/bin/node')
    r = run / 'proxy'; r.mkdir(); f = Bridge(r, h)
    d = LinuxEmulator('/home/art/Android/Sdk', run / 'emulator', hardware_keyboard=False)
    d.start_emulator()
    d.call('shell', 'settings', 'put', 'secure', 'show_ime_with_hard_keyboard', '1')
    require(hashlib.sha256(args.apk.read_bytes()).hexdigest() == result['apkSha256'], 'APK changed while test booted')
    d.install(args.apk); d.reverse(int(f.origin.rsplit(':', 1)[1]))
    s = Matrix(d, f, args.apk, run, []); s.reset(); a = s.app
    a.manage(); s.add(); s.close_manage()
    if args.history:
        a.tap('runtime-shell-'+a.profiles()[0]['id']);a.tap('history-create')
        shell_name=runtime_work_name(a)
    else:
        s.shell('keyboard-anchor');shell_name='keyboard-anchor'
    a.evaluate(OBSERVE)
    if args.history:
        a.evaluate(r'''(()=>{
          const c=window.__TAURI_INTERNALS__.callbacks,set=c.set.bind(c);
          c.set=(id,cb)=>set(id,typeof cb==='function'?v=>{
            const e=v?.message??v;
            if(e?.readerId&&['position','viewport','reading'].includes(e.kind)){
              window.__pressure.positions.push(e);window.__pressure.positions=window.__pressure.positions.slice(-64);
            }return cb(v);
          }:cb);
        })()''')
    script = "for i in range(500):print('\\x1b[32mROW-%04d stable line\\x1b[0m'%i)\nprint('IME_END',flush=True)"
    s.compose('python3 -c ' + shlex.quote(script))
    wait(lambda: s.contains(shell_name, 'IME_END'), 30)
    d.key(4)
    wait(lambda: a.evaluate('window.visualViewport.height') > 800, 15)
    wait(lambda: a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('IME_END'))"), 15)
    time.sleep(.4)
    d.call('shell', 'input', 'swipe', '200', '300', '200', '680', '200')
    wait(lambda: a.evaluate("window.__pressure.positions.some(p=>p.kind==='viewport'&&!p.atBottom)"), 15)

    def sample():
        before = a.evaluate('window.__pressure.positions.length')
        a.evaluate('''(()=>{
          const records=structuredClone(window.__scrollTest.records),r=records[0];
          if(r?.text.length)r.styles=[[0,r.text.length,0xdddddd,-1,4]];
          const b=document.querySelector('.native-terminal-host').getBoundingClientRect();
          return window.__TAURI_INTERNALS__.invoke('plugin:mobile-history|update',{
            readerId:window.__scrollTest.readerId,records,visible:true,followLatest:false,
            sourceEpoch:window.__pressure.positions.filter(p=>p.kind==='position').at(-1)?.sourceEpoch,
            bounds:{x:b.x,y:b.y,width:b.width,height:b.height,viewportWidth:innerWidth}});
        })()''')
        wait(lambda: a.evaluate('window.__pressure.positions.length') > before, 10)
        return a.evaluate("window.__pressure.positions.filter(p=>p.kind==='position').at(-1)")

    def touch_button(testid):
        # Chromium can clip the accessibility WebView vertically during IME
        # changes. CSS pixel scaling remains the physical width scaling.
        # Verify the actual DOM hit before a real Android pointer tap.
        css='[data-testid='+testid+']'
        point=a.evaluate('''(()=>{const e=document.querySelector('''+json.dumps(css)+''');
          e.scrollIntoView({block:'nearest',inline:'nearest'});
          const r=e.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;
          const hit=document.elementFromPoint(x,y);
          return {x:x-visualViewport.offsetLeft,y:y-visualViewport.offsetTop,width:visualViewport.width,hit:hit===e||e.contains(hit)};})()''')
        require(point['hit'],'keyboard fixture touch is obstructed')
        b=d.webview_bounds();scale=(b[2]-b[0])/point['width']
        x=round(b[0]+point['x']*scale);y=round(b[1]+point['y']*scale)
        require(0<x<412 and 24<y<915,'keyboard fixture touch outside owned display')
        d.call('shell','input','tap',str(x),str(y))

    # Match the existing KEY-IME public fixture: a hidden-keyboard quick key
    # suppresses the already-focused editor before the explicit keyboard action.
    touch_button('k-left')
    before = sample(); result['before'] = before; d.screenshot(run / 'before.png')
    touch_button('k-keyboard')
    wait(lambda: 'mInputShown=true' in d.text('shell', 'dumpsys', 'input_method')
         or 'mShowRequested=true' in d.text('shell', 'dumpsys', 'input_method'), 15)
    time.sleep(.7)
    after = sample(); d.screenshot(run / 'keyboard-open.png')
    result.update(before=before, after=after)
    require(after['anchorId'] == before['anchorId']
            and after['anchorCharacter'] == before['anchorCharacter']
            and abs(after['anchorPixel'] - before['anchorPixel']) <= 2,
            'IME changed the actual mid-page reading anchor')
    d.key(4); time.sleep(.7)
    closed = sample(); result['closed'] = closed
    require(closed['anchorId'] == before['anchorId']
            and closed['anchorCharacter'] == before['anchorCharacter']
            and abs(closed['anchorPixel'] - before['anchorPixel']) <= 2,
            'closing IME changed the actual mid-page reading anchor')
    # A reader already following the bottom must keep following it when the
    # keyboard reduces only its bounds, even if no new text arrives.
    for _ in range(6):
        d.call('shell','input','swipe','200','650','200','180','400');time.sleep(.3)
        if a.evaluate("window.__pressure.positions.filter(p=>p.kind==='viewport').at(-1)?.atBottom===true"):break
    wait(lambda:a.evaluate("window.__pressure.positions.filter(p=>p.kind==='viewport').at(-1)?.atBottom===true"),10)
    result['bottomBeforeIme']=True
    touch_button('k-left');touch_button('k-keyboard')
    wait(lambda:'mInputShown=true' in d.text('shell','dumpsys','input_method'),15)
    time.sleep(.7)
    d.screenshot(run/'bottom-keyboard-open.png')
    # Low-velocity, equal opposite physical gestures return to the same
    # actual scroll offset. They expose the existing public viewport event
    # without assigning or mocking Android's scroll position.
    old_count=a.evaluate('window.__pressure.positions.length')
    d.call('shell','input','swipe','200','240','200','260','600');time.sleep(.3)
    d.call('shell','input','swipe','200','260','200','240','600')
    wait(lambda:a.evaluate('window.__pressure.positions.length')>old_count,10)
    time.sleep(.3)
    bottom=a.evaluate("window.__pressure.positions.filter(p=>p.kind==='viewport').at(-1)")
    result['bottomAfterIme']=bottom
    require(bottom and bottom['atBottom'],'opening IME lost bottom-follow while terminal text was unchanged')
    result['passed'] = True
    print('PASS actual native mid-page anchor and bottom-follow across IME open/close', flush=True)
except Exception as e:
    result.update(errorType=type(e).__name__, error=str(e)[:200])
    if d and d.verified: d.screenshot(run / 'failed.png')
    if s and s.app:
        try:
            result['observation'] = s.app.evaluate('window.__pressure')
            result['focus'] = s.app.evaluate('({tag:document.activeElement?.tagName,inputMode:document.activeElement?.inputMode,focused:document.hasFocus(),viewport:visualViewport.height})')
            result['ime'] = [line for line in d.text('shell','dumpsys','input_method').splitlines() if any(k in line for k in ('mInputShown','mShowRequested','mServedView','mActive='))]
        except Exception: pass
    raise
finally:
    errors=[]
    for label, obj, method in [('cdp', s.app if s else None, 'close'), ('android', d if d and d.verified else None, 'cleanup'), ('proxy', f, 'close'), ('runtime', h, 'close'), ('emulator', d, 'shutdown')]:
        if obj:
            try: getattr(obj, method)()
            except Exception as e: errors.append(label+':'+type(e).__name__)
    if short: short.cleanup()
    result['history'] = args.history
    result['cleanupPassed'] = not errors
    if errors: result['cleanupErrors'] = errors
    (run / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    print(run / 'result.json', flush=True)
    if errors: raise RuntimeError('owned fixture cleanup failed')
