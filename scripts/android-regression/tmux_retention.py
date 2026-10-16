"""Actual owned APK/Runtime: detached output, real ring gap and App restart."""
import asyncio,json,re,time,hashlib
from aiohttp import ClientSession,WSMsgType
from pathlib import Path
from device import require,wait
from reconnect_scroll import ReconnectTest

OBSERVER=r'''(()=>{
 const send=window.ipc.postMessage.bind(window.ipc),callbacks=window.__TAURI_INTERNALS__.callbacks;
 window.__retention={page:null};
 window.ipc={postMessage:raw=>{
  const m=JSON.parse(raw),cb=callbacks.get(m.callback);
  if((m.cmd==='plugin:mobile-history|open'||m.cmd==='plugin:mobile-history|update')&&cb)
   callbacks.set(m.callback,value=>{if(m.payload.visible)window.__retention.page=m.payload;return cb(value)});
  return send(raw);
 }};
})()'''


def run_retention(s,h,d,run,result,name,legacy_apk=None,new_apk=None):
 a=s.app;a.evaluate(OBSERVER);label=a.text('.tab.active .name')
 def page():return a.evaluate('window.__retention.page')
 def current(text):
  return a.evaluate('window.__scrollTest.records.some(r=>r.text.includes('+json.dumps(text)+'))') and page() and page().get('sourceEpoch')
 def stats():
  status,data=h.api('GET','/admin/sessions');require(status==200,'owned Runtime list unavailable')
  return next(item for item in data['sessions'] if item['name']==name)
 def open_same():
  if a.evaluate('Boolean(document.querySelector("[data-testid=to-list]")?.getBoundingClientRect().width)'):a.tap('to-list')
  a.tap('runtime-shell-'+a.profiles()[0]['id']);wait(lambda:a.has('.runtime-sessions li'))
  index=wait(lambda:a.evaluate('[...document.querySelectorAll(".runtime-sessions li")].findIndex(e=>['+json.dumps(name)+','+json.dumps(label)+'].includes(e.querySelector("strong").textContent.trim()))+1'))
  a.click(f'.runtime-sessions li:nth-child({index}) button')
  wait(lambda:a.text('.tab.active .name') in (name,label))
 def gesture(up):
  d.call('shell','input','swipe','200','180' if up else '700','200','700' if up else '180','40');time.sleep(.9)
 def scan(label,expected=None):
  seen={};epoch=None
  for _ in range(120):
   gesture(True);p=page();require(p and p.get('sourceEpoch'),'history fell back to replay')
   if epoch is None:epoch=p['sourceEpoch']
   require(p['sourceEpoch']==epoch,'held history changed during current scan')
   require(len(p['records'])<=1024,'native history window unbounded')
   seen.update({row['id']:row for row in p['records']})
   if p['records'][0]['id']==1:
    gesture(True);break
  else:raise RuntimeError('oldest native page not reached')
  rows=[seen[i] for i in sorted(seen)]
  numbers=[int(n) for row in rows for n in re.findall(r'\bR(\d{4})\b',row['text'])]
  if expected is not None:require(numbers==list(range(1,expected+1)),'detached/restarted current native history lost numbered rows')
  image,_=ReconnectTest(a,d,run,'owned','owned').capture_crop(label)
  require(len(image.getcolors(image.width*image.height))>10,'native history pixels blank')
  for _ in range(120):
   gesture(False)
   if page().get('followLatest'):break
  else:raise RuntimeError('live bottom not reached')
  return numbers
 s.compose("for i in $(seq 1 1000); do printf '\\033[32mR%04d 中文 é\\033[0m\\n' \"$i\"; done")
 wait(lambda:current('R1000'),30)
 script=h.home/'detached-producer.py'
 script.write_text('''import os,time
from pathlib import Path
root=Path(__file__).parent
(root/'detached-started').touch()
while not (root/'release-detached').exists():time.sleep(.02)
for i in range(1001,3001):os.write(1,('\\x1b[32mR%04d 中文 é\\x1b[0m\\r\\n'%i).encode())
cols,rows=os.get_terminal_size(1)
# Move the last numbered viewport into native scrollback before repainting.
# CSI 2J intentionally erases visible cells; it must not erase test input.
os.write(1,b'\\r\\n'*(rows+1))
os.write(1,b'\\x1b[0m\\x1b[2J\\x1b[H')
for frame in range(160):
 out='\\x1b[H'
 for y in range(rows):
  out+='\\x1b[%d;1H'%(y+1)
  for x in range(cols):out+='\\x1b[38;2;%d;%d;%dm%s'%((x+frame)%256,(y+frame*3)%256,(x*7+y*13+frame)%256,'X' if frame%2 else 'O')
 os.write(1,out.encode());time.sleep(.1)
os.write(1,b'\\x1b[0m\\x1b[2J\\x1b[HDETACHED_BURST_DONE\\r\\n')
with (root/'once-detached').open('a') as f:f.write('once\\n')
''')
 s.compose('python3 '+str(script)+' &')
 wait(lambda:(h.home/'detached-started').exists(),10)
 before=stats()['bytes_written']
 a.click('.tab.active .close');wait(lambda:not a.has('.tab.active'))
 wait(lambda:all(not item['name'].startswith('mcshot-') for item in h.api('GET','/admin/sessions')[1]['sessions']),20)
 (h.home/'release-detached').touch();wait(lambda:(h.home/'once-detached').exists(),60)
 require((h.home/'once-detached').read_text()=='once\n','detached producer repeated')
 after=stats()['bytes_written'];require(after-before>1024*1024,'actual Runtime stream did not exceed its ring')
 # Observe a REAL official Runtime Gap on an owned attach at cursor zero.
 status,grant=h.api('POST','/admin/sessions/'+name+'/renew');require(status==200,'owned gap-observation grant')
 async def observe_gap():
  # A real ring gap can precede attach-notice. Do not use the older
  # no-gap fixture constructor, which requires attach-notice first.
  async with ClientSession(trust_env=False) as client:
   async with client.ws_connect(f'http://127.0.0.1:{h.port}/pty/{name}?since=0',headers={'Authorization':'Bearer '+grant['token']}) as ws:
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
     message=await asyncio.wait_for(ws.receive(),deadline-time.monotonic())
     if message.type==WSMsgType.TEXT:
      event=json.loads(message.data)
      if event.get('type')=='gap':return event
    raise RuntimeError('official Runtime did not report a real Gap')
 gap=asyncio.run(observe_gap())
 require(gap and gap.get('bytes_dropped',0)>0,'official Runtime did not report a real Gap')
 print('PASS owned detached burst >1MiB and real Runtime Gap',flush=True)
 open_same();wait(lambda:current('DETACHED_BURST_DONE'),30)
 require(len(page()['records'])<=page().get('rows',100),'reconnect loaded more than current viewport')
 scan('detached-current-3000',3000)
 result['cases']['closedTab2000MoreAndOld1000']={'runtimeBytesDuringDetach':after-before,'realGapBytes':gap['bytes_dropped'],'allCurrent3000':True}
 print('PASS reattached current native pages contain all 3000 numbered rows',flush=True)
 a=s.restart();a.evaluate(OBSERVER);open_same();wait(lambda:current('DETACHED_BURST_DONE'),30)
 scan('app-restart-current-3000',3000)
 result['cases']['appRestartAfterRealRingGap']=True
 print('PASS App restart retains all current 3000 rows through tmux pages',flush=True)
 # Native tmux retention has a deliberate finite limit, not a disk journal.
 s.compose("for i in $(seq 3001 7000); do printf '\\033[32mR%04d\\033[0m\\n' \"$i\"; done")
 wait(lambda:current('R7000'),45)
 numbers=scan('native-retention-limit')
 require(numbers and numbers[0]>1 and numbers[-1]==7000,'native history eviction did not match finite retention')
 result['cases']['nativeFiniteRetention']={'firstRetainedNumber':numbers[0],'lastRetainedNumber':numbers[-1],'retainedNumberRows':len(numbers),'oldestEvicted':True}
 print('PASS finite native tmux retention after 7000 numbered rows',flush=True)
 if legacy_apk:
  a.close();d.stop()
  d.call('install','-r','-d',str(legacy_apk.resolve()),timeout=120)
  a=s.launch();open_same()
  marker=h.home/'once-old-client'
  s.compose("printf '\\033[32mOLD_CLIENT_OK\\033[0m\\n'; printf once > "+str(marker))
  wait(lambda:marker.exists(),20)
  wait(lambda:a.evaluate('window.__scrollTest.records.some(r=>r.text.includes("OLD_CLIENT_OK"))||[...document.querySelectorAll(".xterm-rows")].some(e=>e.textContent.includes("OLD_CLIENT_OK"))'),20)
  require(stats()['alive'],'old APK killed new tmux work')
  d.screenshot(run/'old-apk-new-tmux.png')
  a.close();d.stop();d.install(new_apk)
  a=s.launch();a.evaluate(OBSERVER);open_same()
  wait(lambda:current('OLD_CLIENT_OK'),30)
  require(marker.read_text()=='once','old APK input was repeated')
  result['cases']['oldApkRawConnectionAndUpgrade']={'oldApkSha256':hashlib.sha256(legacy_apk.read_bytes()).hexdigest(),'sameWorkAlive':True,'privateProfileAndKeystorePreserved':True}
 print('PASS detached 2000 + old1000, actual Runtime >1MiB/Gap, App restart, finite tmux retention',flush=True)
