"""Fixed CLI through actual header/manual UI and current native history pages."""
import json, re, time
from pathlib import Path
from device import STORE, require, wait, runtime_work_name
from reconnect_scroll import ReconnectTest


def configure(s, h, d):
    folder=h.home/'bin';folder.mkdir(exist_ok=True)
    fake=folder/'codex'
    fake.write_text('''#!/usr/bin/python3
import signal,time
from pathlib import Path
root=Path(__file__).resolve().parent.parent
label=(root/'fake-label').read_text().strip()
once=root/('once-'+label)
with once.open('a') as f:f.write('once\\n')
def done(*args):
 print('FAKE-DONE-'+label,flush=True)
 raise SystemExit(0)
signal.signal(signal.SIGINT,done)
print('\\x1b[36mFAKE-REPLY-'+label+' 中文 é\\x1b[0m',flush=True)
while True:time.sleep(.1)
''');fake.chmod(0o700)
    # This is the supported public metadata format used by the existing UI
    # matrix. Keep the native credential ciphertext and profile identity.
    s.app.close();d.stop()
    raw=d.store().decode('utf-8-sig')
    entry='coding_cli = { id = "codex", name = "CODEX", command = '+json.dumps(str(fake))+', args = ["--no-alt-screen"] }'
    updated=raw.replace('category = "host"','category = "host"\n'+entry)
    require(updated!=raw,'fixture profile metadata not found')
    d.private_write(STORE,updated.encode())
    return s.launch()


def run_cycles(s, h, d, run, result, first_name):
    a=s.app
    result['cases']['fixedCliPaths']=[]
    names=[]
    def save():
        (run/'dialogue-progress.json').write_text(json.dumps(result['cases'],ensure_ascii=False,indent=2)+'\n')
    def page():
        return a.evaluate('({epoch:window.__lazy.renderEpoch,follow:window.__lazy.followLatest,records:window.__scrollTest.records})')
    def gesture(older):
        d.call('shell','input','swipe','200','180' if older else '700','200','700' if older else '180','40')
        time.sleep(.9)
    def read_current(label, limit):
        # Rebuild evidence from the current displayed epoch on EVERY cycle.
        # Nothing observed in a previous cycle is used to prove retention.
        seen={};epoch=None
        for attempt in range(40):
            gesture(True)
            value=page()
            require(value.get('epoch'),'native reader silently fell back to local replay')
            if epoch is None:epoch=value['epoch']
            require(value['epoch']==epoch,'source epoch changed while reading older pages')
            rows=value['records'];require(0<len(rows)<=1024,'native window exceeded its bound')
            for row in rows:seen[row['id']]=row
            positions=a.evaluate('window.__pressure.positions.filter(p=>p.kind==="viewport"&&p.sourceEpoch==='+json.dumps(epoch)+')')
            if rows[0]['id']==1 and positions and positions[-1].get('anchorId',999999)<=8:break
        else:raise RuntimeError('native upward gestures did not reach oldest retained page')
        rows=[seen[i] for i in sorted(seen)]
        numbers=[int(n) for row in rows for n in re.findall(r'\bN(\d{4})\b',row['text'])]
        require(numbers==list(range(1,limit+1)),'current native pages lost or duplicated number rows: '+label)
        replies=[row for row in rows if 'FAKE-REPLY-' in row['text']]
        expected=[f'FAKE-REPLY-{label[:2]}-P{i}' for i in range(1,limit//200+1)]
        require([re.search(r'FAKE-REPLY-[HM]\d-P\d',row['text'])[0] for row in replies]==expected,'current native pages lost CLI dialogues')
        require(all(any(style[2]==0x6a9955 for style in row['styles']) for row in rows if re.search(r'\bN\d{4}\b',row['text'])),'number colors missing')
        require(all(any(style[2]!=-1 for style in row['styles']) for row in replies),'CLI dialogue colors missing')
        image,_=ReconnectTest(a,d,run,'owned','owned').capture_crop(label+'-current-oldest')
        green=sum(1 for r,g,b in image.getdata() if g>120 and g>r*1.2 and g>b*1.2)
        require(green>10,'native current history pixels lost green text')
        for _ in range(40):
            gesture(False)
            if page().get('follow'):break
        else:raise RuntimeError('native downward gestures did not return to live bottom')
        return {'numbers':len(numbers),'dialogues':len(replies),'sourceEpoch':epoch,'currentNativePages':True,'greenNativePixels':green}
    for path in ['header','manual']:
        for independent in range(1,4):
            tag=('H' if path=='header' else 'M')+str(independent)
            if names:
                a.click('.tab.active .close');wait(lambda:not a.has('.tab.active'))
                a.tap('to-list');a.tap('runtime-shell-'+a.profiles()[0]['id']);a.tap('history-create')
            name=runtime_work_name(a,names[-1] if names else None)
            require(name.startswith('th2-') and name not in names,'independent test reused a work Shell')
            names.append(name)
            wait(lambda:page().get('epoch'),30)
            case={'path':path,'independent':independent,'name':name,'cycles':[]}
            result['cases']['fixedCliPaths'].append(case)
            for cycle in range(1,6):
                label=tag+'-P'+str(cycle)
                (h.home/'fake-label').write_text(label)
                before=a.evaluate('window.__lazy.olderCount')
                start=(cycle-1)*200+1;end=cycle*200
                s.compose(f"for i in $(seq {start} {end}); do printf '\\033[32mN%04d 中文 é\\033[0m\\n' \"$i\"; done")
                wait(lambda:a.evaluate('window.__scrollTest.records.some(r=>r.text.includes('+json.dumps(f'N{end:04d}')+'))'),30)
                if path=='header':
                    require(a.has('[data-testid=launch-coding-cli]'),'configured CODEX header button absent')
                    a.tap('launch-coding-cli')
                else:s.compose(str(h.home/'bin/codex')+' --no-alt-screen')
                wait(lambda:a.evaluate('window.__scrollTest.records.some(r=>r.text.includes('+json.dumps('FAKE-REPLY-'+label)+'))'),30)
                a.tap('k-ctrlc')
                wait(lambda:a.evaluate('window.__scrollTest.records.some(r=>r.text.includes('+json.dumps('FAKE-DONE-'+label)+'))'),30)
                require((h.home/('once-'+label)).read_text()=='once\n','CLI launch was replayed')
                require(a.evaluate('window.__lazy.olderCount')==before,'history prefetched without an upward gesture')
                evidence=read_current(label,end);case['cycles'].append(evidence);save()
                print('PASS current native '+label+': '+str(end)+' numbers + '+str(cycle)+' dialogues, color/pixels',flush=True)
            d.screenshot(run/(tag+'-complete.png'))
    result['cases']['threeHeaderAndThreeManualComplete']=True
    return names[-1]
