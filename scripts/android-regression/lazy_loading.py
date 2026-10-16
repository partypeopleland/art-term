"""Actual APK and owned official Runtime: Host tmux pages only on upward touch."""
import argparse, base64, hashlib, json, shlex, shutil, tempfile, time, uuid
from pathlib import Path
from connection_pressure import ROOT, Harness, Bridge, Matrix, LinuxEmulator, OBSERVE
from device import require, wait, runtime_work_name

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--apk', type=Path, required=True)
p.add_argument('--dialogues', action='store_true', help='three independent header and manual fixed CLI cycles each')
p.add_argument('--retention', action='store_true', help='detached output, real Runtime Gap, App restart and finite tmux retention')
p.add_argument('--legacy-apk', type=Path, help='owned old-client compatibility and upgrade check after retention')
args = p.parse_args()
run = ROOT/'target/android-regression'/('lazy-'+uuid.uuid4().hex); run.mkdir(mode=0o700)
runtime_dir = tempfile.TemporaryDirectory(prefix='th-')
h=f=d=s=None
result={'passed':False,'apkSha256':hashlib.sha256(args.apk.read_bytes()).hexdigest(),'cases':{}}
try:
    h=Harness(Path(runtime_dir.name),Path('/home/art/.local/share/my-cli/openab-pty/openab-pty'))
    # The private binary is executed directly; fixture PATH contains no Node.
    node=ROOT/'target/android-tools/node-v22.23.3-linux-x64/bin'
    support=h.helper/'tmux-v2';support.mkdir()
    for source,name in [(ROOT/'scripts/openab-runtime/tmux-v2/tmux-shell.py','tmux-shell.py'),(ROOT/'target/tmux-history/tmux-v2/tmux-history.cjs','tmux-history.cjs'),(ROOT/'target/tmux-lazy-proof/tmux-3.6a/tmux','tmux')]:
        shutil.copyfile(source,support/name);(support/name).chmod(0o700)
    (support/'tmux').rename(support/'tmux-real')
    (support/'tmux').write_text('''#!/usr/bin/python3
import subprocess,sys,json,base64,shlex
from pathlib import Path
base=Path(__file__).parent
data=sys.stdin.buffer.read() if 'source-file' in sys.argv else None
p=subprocess.run([str(base/'tmux-real'),*sys.argv[1:]],input=data,capture_output=True)
args=sys.argv[1:]
if data:
 args=[]
 for line in data.decode().splitlines():args.extend([*shlex.split(line),';'])
if 'capture-pane' in args:
 with (base/'capture-observer.jsonl').open('a') as f:f.write(json.dumps({'args':args,'stdout':base64.b64encode(p.stdout).decode(),'returncode':p.returncode})+'\\n')
sys.stdout.buffer.write(p.stdout);sys.stderr.buffer.write(p.stderr);sys.exit(p.returncode)
''');(support/'tmux').chmod(0o700)
    (h.helper/'node').symlink_to(node/'node')
    proxy_root=run/'proxy';proxy_root.mkdir()
    f=Bridge(proxy_root,h)
    d=LinuxEmulator('/home/art/Android/Sdk',run/'emulator');d.start_emulator()
    require(hashlib.sha256(args.apk.read_bytes()).hexdigest()==result['apkSha256'],'APK changed during test boot')
    d.install(args.apk);d.reverse(int(f.origin.rsplit(':',1)[1]))
    s=Matrix(d,f,args.apk,run,[]);s.reset();a=s.app
    a.manage();s.add();s.close_manage()
    if args.dialogues:
        from tmux_dialogues import configure, run_cycles
        a=configure(s,h,d)
    a.evaluate(OBSERVE)
    a.evaluate(r'''(()=>{
      const callbacks=window.__TAURI_INTERNALS__.callbacks,set=callbacks.set.bind(callbacks);
      callbacks.set=(id,cb)=>set(id,typeof cb==='function'?v=>{
        const e=v?.message??v;
        if(e?.readerId&&(e.kind==='viewport'||e.kind==='position'||e.kind==='reading')){
          window.__pressure.positions.push(e);window.__pressure.positions=window.__pressure.positions.slice(-64);
        }
        return cb(v);
      }:cb);
      window.__lazy={requests:[],replies:[],olderCount:0,viewportCount:0};
      const send=window.ipc.postMessage.bind(window.ipc);
      window.ipc={postMessage:raw=>{
        const m=JSON.parse(raw),cb=callbacks.get(m.callback);
        if((m.cmd==='plugin:mobile-history|open'||m.cmd==='plugin:mobile-history|update')&&cb)
          callbacks.set(m.callback,v=>{window.__lazy.renderEpoch=m.payload.sourceEpoch;window.__lazy.followLatest=m.payload.followLatest;return cb(v)});
        if(m.cmd==='gui_runtime_history'){
          if(m.payload.request.op==='older')window.__lazy.olderCount++;
          if(m.payload.request.op==='viewport')window.__lazy.viewportCount++;
          window.__lazy.requests.push({...m.payload.request});
          window.__lazy.requests=window.__lazy.requests.slice(-256);
          if(cb)callbacks.set(m.callback,v=>{
            window.__lazy.replies.push({kind:v.kind,error:v.error,instance:v.instance,epoch:v.epoch,rowCount:v.rowCount,start:v.start,end:v.end,records:v.records?.length});
            window.__lazy.replies=window.__lazy.replies.slice(-256);return cb(v);
          });
        }
        return send(raw);
      }};
    })()''')
    a.tap('runtime-shell-'+a.profiles()[0]['id'])
    require(not a.has('.runtime-sessions input'),'Shell creation still requires a name')
    require(a.evaluate('[...document.querySelectorAll(".runtime-sessions button")].filter(b=>b.textContent==="建立並連線").length')==1,'Shell creation has multiple choices')
    require(not a.evaluate('document.querySelector(".runtime-sessions").textContent.includes("建立主機歷史 Shell")'),'separate history create option remains')
    a.tap('history-create')
    name=runtime_work_name(a)
    display_name=wait(lambda:a.text('.tab.active .name') if a.has('.tab.active .name') else None)
    require(__import__('re').fullmatch(r's-[0-9a-f]{8}',display_name),'automatic Shell label is not short')
    result['cases']['singleAutomaticTmuxCreate']={'label':display_name,'nameInput':False,'oneCreateButton':True}
    require(name.startswith('th2-'),'explicit history creation did not select paged tmux work')
    wait(lambda:a.evaluate("window.__pressure.positions.some(p=>p.sourceEpoch)"),30)
    initial=a.evaluate("window.__lazy.replies.find(r=>r.kind==='page')")
    require(initial and initial['records']<=100,'initial phone load was not just its viewport')
    require(not a.evaluate("window.__lazy.requests.some(r=>r.op==='older')"),'older history was fetched without an upward gesture')
    result['cases']['initialViewportOnly']=initial
    if args.retention:
        from tmux_retention import run_retention
        run_retention(s,h,d,run,result,name,args.legacy_apk,args.apk)
        result['passed']=True
        raise SystemExit(0)
    if args.dialogues:
        run_cycles(s,h,d,run,result,name)
        result['passed']=True
        print('PASS actual APK three CODEX header + three manual fixed CLI suites',flush=True)
        raise SystemExit(0)

    def produce(start,count,label):
        code=f'''import os
for i in range({start},{start+count}):
 print("\\x1b[32mHIST-%05d 中文 é"%i+("界"*40 if i%17==0 else "")+"\\x1b[0m",flush=True)
with open("once-{label}","a") as f:f.write("once\\n")
print("FINISHED_{label}",flush=True)
'''
        command='python3 -c '+shlex.quote("exec(__import__('base64').b64decode("+repr(base64.b64encode(code.encode()).decode())+'))')
        s.compose(command)
        wait(lambda:(h.home/('once-'+label)).exists(),30)
        require((h.home/('once-'+label)).read_text()=='once\n','producer input was repeated')

    for phase in range(5):
        produce(phase*200,200,'phase'+str(phase))
        wait(lambda:a.evaluate('window.__scrollTest.records.some(r=>r.text.includes('+json.dumps('FINISHED_phase'+str(phase))+'))'),30)
    require(not a.evaluate("window.__lazy.requests.some(r=>r.op==='older')"),'live refresh preloaded older records')
    require(not a.evaluate('window.__pressure.ended'),'normal live output disconnected')
    result['cases']['fiveRounds200Lines']=True
    initial_count=a.evaluate('window.__scrollTest.records.length')
    for _ in range(6):
        d.call('shell','input','swipe','200','180','200','650','180');time.sleep(.5)
        if a.evaluate('window.__scrollTest.records.length')>initial_count:break
    wait(lambda:a.evaluate('window.__scrollTest.records.length')>initial_count,15)
    require(a.evaluate("window.__lazy.requests.some(r=>r.op==='older'&&r.limit===256)"),'upward touch did not fetch Host older page')
    require(a.evaluate('window.__scrollTest.records.length')<=1024,'phone history window exceeded cap')
    wait(lambda:a.evaluate('''window.__pressure.positions.some((v,i)=>v.kind==='viewport'&&v.sourceEpoch&&window.__pressure.positions.slice(i+1).some(p=>
      p.kind==='position'&&p.sourceEpoch===v.sourceEpoch&&p.firstId<v.firstId&&p.restoredAnchorId===v.anchorId&&p.restoredAnchorCharacter===v.anchorCharacter&&Math.abs(p.restoredAnchorPixel-v.anchorPixel)<=2))'''),15)
    require(a.evaluate('window.__scrollTest.records.some(r=>r.styles.some(s=>s[2]===0x6a9955))'),'Host page lost terminal color')
    frozen=a.evaluate('JSON.stringify(window.__scrollTest.records)')
    viewport_count=a.evaluate("window.__lazy.viewportCount")
    produce(1000,200,'while-reading');time.sleep(.7)
    require(a.evaluate('JSON.stringify(window.__scrollTest.records)')==frozen,'live output replaced the page being read')
    require(a.evaluate("window.__lazy.viewportCount")==viewport_count,'reading old page fetched a different viewport epoch')
    result['cases']['olderColorAndPixelAnchor']=True
    result['cases']['liveWhileReadingPreserved']=True
    # Closing a tab must remove only its hidden helper. The tmux work stays alive.
    a.click('.tab.active .close');wait(lambda:not a.has('.tab.active'))
    wait(lambda:all(not item['name'].startswith('mcshot-') for item in h.api('GET','/admin/sessions')[1]['sessions']),20)
    require(any(item['name']==name and item['alive'] for item in h.api('GET','/admin/sessions')[1]['sessions']),'closing history reader killed its work')
    a.tap('to-list');a.tap('runtime-shell-'+a.profiles()[0]['id']);wait(lambda:a.has('.runtime-sessions li'))
    a.click_text('接管連線' if a.evaluate("[...document.querySelectorAll('.runtime-sessions li button')].some(b=>b.textContent==='接管連線')") else '連線','.runtime-sessions li button')
    wait(lambda:a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('FINISHED_while-reading'))"),30)
    require(a.evaluate('window.__scrollTest.records.length')<=100,'reconnect preloaded full Host history')
    result['cases']['reconnectViewportAndOwnedCleanup']=True
    code="import os\nfor i in range(80000):os.write(1,('\\x1b[32mBULK-%06d '%i+'x'*40+'\\x1b[0m\\n').encode())\nwith open('once-BULK','a') as f:f.write('once\\n')\nprint('FINISHED_BULK',flush=True)"
    older_before=a.evaluate("window.__lazy.olderCount")
    s.compose('python3 -c '+shlex.quote("exec(__import__('base64').b64decode("+repr(base64.b64encode(code.encode()).decode())+'))'))
    wait(lambda:(h.home/'once-BULK').exists(),60)
    wait(lambda:a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('FINISHED_BULK'))"),90)
    require((h.home/'once-BULK').read_text()=='once\n','bulk producer input replayed')
    require(not a.evaluate('window.__pressure.ended'),'normal tmux bulk output disconnected')
    require(a.evaluate('window.__scrollTest.records.length')<=100,'bulk output preloaded all Host history')
    require(a.evaluate("window.__lazy.olderCount")==older_before,'bulk output fetched older without a gesture')
    result['cases']['innerProducerOver4MiB']=True
    # Separate work names must have separate snapshots and private tmux sockets.
    a.tap('to-list');a.tap('runtime-shell-'+a.profiles()[0]['id']);a.tap('history-create')
    second=runtime_work_name(a,name)
    require(second!=name,'second work reused the first native binding')
    require(second.startswith('th2-'),'second history Shell creation failed')
    s.compose("printf 'SECOND_ONLY\\n'")
    wait(lambda:a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('SECOND_ONLY'))"),30)
    require(not a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('FINISHED_phase'))"),'second Shell inherited first history')
    s.tab(display_name)
    wait(lambda:a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('FINISHED_BULK'))"),30)
    require(not a.evaluate("window.__scrollTest.records.some(r=>r.text.includes('SECOND_ONLY'))"),'first Shell inherited second history')
    result['cases']['twoWorkShellIsolation']=True
    result['cases']['officialRuntimeWithoutHistoryProxy']=True
    result['passed']=True;d.screenshot(run/'passed.png');print('PASS actual APK Host lazy loading, five rounds, color, pixel anchor, live hold, reconnect viewport',flush=True)
except Exception as e:
    result.update(errorType=type(e).__name__,error=str(e)[:250])
    if s and s.app:
        try:result['lazy']=s.app.evaluate('window.__lazy');result['native']=s.app.evaluate('window.__pressure')
        except Exception:pass
    if d and d.verified:d.screenshot(run/'failed.png')
    raise
finally:
    errors=[]
    if h and (h.helper/'tmux-v2/capture-observer.jsonl').exists():
        shutil.copyfile(h.helper/'tmux-v2/capture-observer.jsonl',run/'capture-observer.jsonl')
    for label,obj,method in [('cdp',s.app if s else None,'close'),('android',d if d and d.verified else None,'cleanup'),('proxy',f,'close'),('runtime',h,'close'),('emulator',d,'shutdown')]:
        if obj:
            try:getattr(obj,method)()
            except Exception as e:errors.append(label+':'+type(e).__name__)
    runtime_dir.cleanup();result['cleanupPassed']=not errors
    if errors:result['cleanupErrors']=errors
    (run/'result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n');print(run/'result.json',flush=True)
    if errors:raise RuntimeError('owned cleanup failed')
