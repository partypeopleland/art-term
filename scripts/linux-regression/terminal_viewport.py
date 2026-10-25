#!/usr/bin/env python3
"""Owned official Runtime/native viewport regression. Candidate helpers only; no live install."""
import argparse,base64,importlib.util,json,os,re,shutil,signal,socket,subprocess,tempfile,time,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('native_view_harness',ROOT/'scripts/linux-regression/run.py');linux=importlib.util.module_from_spec(spec);spec.loader.exec_module(linux)

def require(value,message):
    if not value: raise AssertionError(message)

class Reader:
    def __init__(self,h,td,key):
        self.h=h;self.nonce=uuid.uuid4().hex;self.id=0;self.buffer=b''
        self.name,grant=h.create('mcshot-');self.ws=h.attach(self.name,grant)
        self.ws.send(2,f'exec {h.helper}/node {td}/tmux-history.cjs {key} {self.nonce}\n'.encode())
        self.ready=self.receive(0)
    def receive(self,request_id,timeout=12):
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            while b'\n' in self.buffer:
                line,self.buffer=self.buffer.split(b'\n',1)
                try: obj=json.loads(line.strip())
                except (ValueError,UnicodeDecodeError):continue
                if obj.get('nonce')==self.nonce and obj.get('requestId')==request_id:return obj
            op,raw=self.ws.recv(deadline-time.monotonic())
            if op==2:self.buffer+=raw
            elif op==8:raise AssertionError('owned history helper closed before reply')
            require(len(self.buffer)<512*1024,'helper frame bound')
        raise AssertionError('helper reply deadline')
    def rpc(self,op,**kwargs):
        self.id+=1;self.ws.send(2,(json.dumps({'v':1,'nonce':self.nonce,'requestId':self.id,'op':op,**kwargs})+'\n').encode());return self.receive(self.id)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--helper',type=Path,default=ROOT/'target/inline-history/helper/tmux-history.cjs');parser.add_argument('--red',action='store_true');parser.add_argument('--node',type=Path,default=Path.home()/'.local/share/art-term/openab-pty/node');parser.add_argument('--tmux',type=Path,default=Path.home()/'.local/share/art-term/openab-pty/tmux-v2/tmux');args=parser.parse_args()
    output=ROOT/'target/inline-history'/('runtime-'+uuid.uuid4().hex);output.mkdir(parents=True,mode=0o700)
    # Unix socket paths are <=108 bytes; use an owned short ephemeral HOME, evidence stays target.
    root=Path(tempfile.mkdtemp(prefix='av-'))
    h=linux.Harness(root,None);result={'passed':False,'officialRuntime':True,'productionTouched':False,'cases':{}}
    try:
        td=h.helper/'tmux-v2';td.mkdir(mode=0o700)
        require(subprocess.check_output([str(args.tmux),'-V']).strip()==b'tmux 3.6a','requires tmux3.6a')
        require(subprocess.check_output([str(args.node),'--version']).startswith(b'v22.'),'requires Node22')
        shutil.copyfile(args.tmux,td/'tmux');(td/'tmux').chmod(0o700)
        shutil.copyfile(args.node,h.helper/'node');(h.helper/'node').chmod(0o700)
        shutil.copyfile(args.helper,td/'tmux-history.cjs')
        (h.home/'.bashrc').write_text("PS1='WORK_READY> '\n")
        key=uuid.uuid4().hex[:24];work,grant=h.create('work-');ws=h.attach(work,grant)
        ws.send(2,f'exec python3 {ROOT}/scripts/openab-runtime/tmux-v2/tmux-shell.py {key}\n'.encode())
        bootstrap=bytearray();until=time.monotonic()+5
        while b'WORK_READY> ' not in bootstrap:
            opcode,raw=ws.recv(until-time.monotonic())
            if opcode==2:bootstrap.extend(raw)
            if opcode==8:raise AssertionError('owned bootstrap closed: '+bytes(bootstrap[-2000:]).decode('utf-8','replace'))
        state=h.home/'.local/share/art-term/tmux-history-v2'/key;sock=state/'tmux.sock'
        end=time.monotonic()+5
        while not sock.exists() and time.monotonic()<end:time.sleep(.02)
        def tmux(*params):return subprocess.check_output([str(td/'tmux'),'-S',str(sock),*params],timeout=3).decode().strip()
        def mode():return tmux('display-message','-p','-t','work:0.0','#{pane_in_mode}|#{scroll_position}|#{@art_term_native_view}')
        pty=bytearray(bootstrap)
        def drain():
            data=bytearray();until=time.monotonic()+1
            while time.monotonic()<until:
                try:op,raw=ws.recv(min(.15,until-time.monotonic()))
                except (socket.timeout,TimeoutError):break
                if op==2:data.extend(raw)
                elif op==8:raise AssertionError('work PTY unexpectedly closed')
            pty.extend(data);return bytes(data)
        node_projection = """const fs=require('fs');const {Terminal}=require('./apps/client/node_modules/@xterm/xterm');const input=JSON.parse(fs.readFileSync(0,'utf8'));const t=new Terminal({cols:80,rows:24,scrollback:5000,allowProposedApi:true});t.write(Buffer.from(input,'base64'),()=>{const b=t.buffer.active;const rows=Array.from({length:24},(_,i)=>{const l=b.getLine(b.viewportY+i),c=l?.getCell(0);return {text:l?.translateToString(true)||'',fg:c?.getFgColor(),fgMode:c?.getFgColorMode()}});console.log(JSON.stringify(rows));t.dispose()});"""
        def rendered():
            raw=subprocess.check_output([str(args.node),'-e',node_projection],input=json.dumps(base64.b64encode(pty).decode()).encode(),cwd=ROOT,timeout=5)
            return json.loads(raw)
        def work_grid(native=False):return tmux('capture-pane','-p',*(['-M'] if native else []),'-t','work:0.0')
        def wait_grid(marker):
            until=time.monotonic()+6
            while time.monotonic()<until:
                if marker in work_grid():return
                time.sleep(.03)
            raise AssertionError('work output sentinel did not complete')
        ws.send(2,b"printf '\\033[38;2;43;157;211mROW-%04d \\346\\270\\254\\350\\251\\246\\033[0m\\n' $(seq 1 1200); printf 'OUTPUT_%s\\n' READY\n")
        wait_grid('OUTPUT_READY');drain()
        def bracketed_mode():
            flags=re.findall(rb'\x1b\[\?2004([hl])',pty)
            return flags[-1].decode() if flags else 'unset'
        result['bracketedPasteLive']=bracketed_mode()
        live=rendered();live_row=next(row for row in live if row['text'].startswith('ROW-'))
        result['liveFirstRow']=live_row
        reader=Reader(h,td,key)
        require(reader.ready.get('type')=='ready','candidate helper did not reach READY')
        require(reader.ready.get('nativeViewport')==1,'old helper has no neutral native viewport capability')
        def helper_pid(r):
            for entry in Path('/proc').iterdir():
                if not entry.name.isdigit():continue
                try:
                    fields=(entry/'stat').read_text().rsplit(') ',1)[1].split()
                    if int(fields[1])==h.process.pid and r.nonce.encode() in (entry/'cmdline').read_bytes():return int(entry.name)
                except (FileNotFoundError,PermissionError,ProcessLookupError):pass
            raise AssertionError('owned Node helper process not found')
        def guard_pid(r):
            children=Path(f'/proc/{helper_pid(r)}/task/{helper_pid(r)}/children').read_text().split()
            require(len(children)==1,'owned Node should have one pipe guard')
            return int(children[0])
        # Android-only five-operation flow must not spawn a native guard or require Python mode control.
        android=Reader(h,td,key)
        snapshot=android.rpc('viewport');require(snapshot['type']=='page','old viewport unsupported')
        android.rpc('ack',epoch=snapshot['epoch']);android.rpc('keep',epoch=snapshot['epoch'])
        older=android.rpc('older',epoch=snapshot['epoch'],before=snapshot['start'],limit=256)
        require(older['type']=='page' and older['end']==snapshot['start'],'old older boundary changed')
        require(not Path(f'/proc/{helper_pid(android)}/task/{helper_pid(android)}/children').read_text().strip(),'Android-only snapshot spawned guard')
        require(android.rpc('close')['type']=='ack','old close ACK missing')
        android=Reader(h,td,key);require(android.rpc('viewport')['type']=='page','snapshot reopen failed');android.rpc('close')
        result['cases']['oldFiveOperationCloseReopenAndLazyGuard']=True
        # Separate private work namespace is also the real 120s no-API idle lease proof.
        idle_key=uuid.uuid4().hex[:24];idle_work,idle_grant=h.create('work-');idle_ws=h.attach(idle_work,idle_grant)
        idle_ws.send(2,f'exec python3 {ROOT}/scripts/openab-runtime/tmux-v2/tmux-shell.py {idle_key}\n'.encode());idle_ws.collect(b'WORK_READY> ',5)
        idle_sock=h.home/'.local/share/art-term/tmux-history-v2'/idle_key/'tmux.sock'
        def idle_mode():return subprocess.check_output([str(td/'tmux'),'-S',str(idle_sock),'display-message','-p','-t','work:0.0','#{pane_in_mode}|#{@art_term_native_view}'],timeout=3).decode().strip()
        idle=Reader(h,td,idle_key)
        # Empty up must return live rather than stick in copy-mode.
        empty=idle.rpc('scroll',lines=-34);require(not empty['held'],'empty native history stuck in copy-mode')
        result['cases']['emptyHistoryUpReturnsLive']=True
        idle_ws.send(2,b"printf 'IDLE-%s\\n' $(seq 1 100); printf 'IDLE_%s\\n' READY\n");idle_ws.collect(b'IDLE_READY',5)
        require(idle.rpc('scroll',lines=-3)['held'],'idle proof did not hold')
        idle_started=time.monotonic()
        before=mode();status=reader.rpc('scroll',lines=0)
        require(status['type']=='view' and not status['held'] and mode()==before,'status claimed or repainted work')
        ws.send(2,b"(sleep 2; for i in $(seq 1201 1240); do printf 'LIVE-%04d\\n' \"$i\"; sleep .02; done; printf 'BG_%s\\n' DONE) &\n")
        drain()
        up=reader.rpc('scroll',lines=-34);drain()
        require(up.get('type')=='view' and up['held'] and up['offset']>=34,'native up did not hold original work pane')
        result['bracketedPasteHeld']=bracketed_mode()
        view=rendered();old_row=next(row for row in view if row['text'].startswith('ROW-'))
        require(int(old_row['text'][4:8]) < int(live_row['text'][4:8]),'fresh original PTY frame did not render older rows')
        require(old_row['fg']==live_row['fg'] and old_row['fgMode']==live_row['fgMode'] and '測試' in old_row['text'],'native PTY changed original style/Unicode')
        result['nativeUpFirstRow']=old_row
        for _ in range(4):up=reader.rpc('scroll',lines=-256)
        drain();view=rendered();old_row=next(row for row in view if row['text'].startswith('ROW-'))
        require(up['held'] and int(old_row['text'][4:8])<201,'original rendered viewport did not reach 1000 rows earlier')
        result['cases']['originalPtyStyledUnicode1000Rows']=True
        # Real background output continues while copy-mode holds the original rendered rows.
        held=[row['text'] for row in view]
        ws.send(2,b'\x1b[1;1R');drain()
        require(mode().startswith('1|'),'outer terminal CPR response left held mode')
        result['cases']['outerProtocolReplyDoesNotFollow']=True
        # Start via the owned inner pane only for fixture workload; never a product Shell command.
        wait_grid('BG_DONE');drain()
        require([row['text'] for row in rendered()]==held,'continuous output pulled original renderer away from held history')
        for _ in range(24):
            down=reader.rpc('scroll',lines=256)
            if not down['held']:break
        drain();require(not down['held'] and any('BG_DONE' in row['text'] for row in rendered()),'down to latest did not immediately show current original PTY output')
        result['cases']['backgroundOutputHeldAndNaturalLatest']=True
        reader.rpc('scroll',lines=-34);drain()
        second=Reader(h,td,key);foreign=second.rpc('scroll',lines=-1)
        require(foreign.get('kind')=='busy','another reader took owned mode')
        second.rpc('close');require(mode().startswith('1|'),'foreign reader close cancelled owned mode')
        result['cases']['samePaneForeignReaderProtection']=True
        pid=int(mode().split('|')[2].split(':')[0]);start=Path(f'/proc/{pid}/stat').read_text().rsplit(') ',1)[1].split()[19]
        # A command is injected only through the atomic native held-write operation.
        literal="printf 'HELD_INPUT_%s\\n' EXACT_ONCE\n".encode()
        follow=reader.rpc('follow',input=base64.b64encode(literal).decode())
        require(follow['type']=='view' and not follow['held'] and follow['offset']==0,'follow did not restore live')
        input_output=ws.collect(b'HELD_INPUT_EXACT_ONCE',4);pty.extend(input_output);drain()
        require(input_output.count(b'HELD_INPUT_EXACT_ONCE')==1,'held input was duplicated')
        result['cases']['atomicFollowLiteralInputExactlyOnce']=True
        # Existing Android snapshot contract remains available on this same endpoint.
        snapshot=reader.rpc('viewport');require(snapshot['type']=='page','old viewport unsupported')
        reader.rpc('ack',epoch=snapshot['epoch']);reader.rpc('keep',epoch=snapshot['epoch'])
        older=reader.rpc('older',epoch=snapshot['epoch'],before=snapshot['start'],limit=256)
        require(older['type']=='page' and older['end']==snapshot['start'],'old older boundary changed')
        result['cases']['oldFiveOperationCompatibility']=True
        # Inner alternate is temporary mode-unavailable, not permanent capability failure.
        ws.send(2,b"printf '\\033[?1049hINNER_ALT\\n'; sleep 1; printf '\\033[?1049lALT_RETURNED\\n'\n")
        ws.collect(b'INNER_ALT',3)
        unavailable=reader.rpc('scroll',lines=-1)
        require(unavailable['type']=='view' and unavailable['available'] is False,'inner alternate was not transient')
        ws.collect(b'ALT_RETURNED',3)
        require(reader.rpc('scroll',lines=-1)['available'],'inner alternate did not recover')
        reader.rpc('follow')
        result['cases']['innerAlternateTransientRecovery']=True
        # Manual, non-owned mode is Busy even for status0; it must not confirm safe input.
        tmux('copy-mode','-t','work:0.0')
        require(reader.rpc('scroll',lines=0).get('kind')=='busy','status0 misclassified manual mode')
        tmux('send-keys','-X','-t','work:0.0','cancel')
        result['cases']['manualModeStatusRejected']=True
        # Runtime DELETE kills its owned PTY process group. Detached guard must outlive it long enough to clean.
        reader.rpc('scroll',lines=-3);guard=guard_pid(reader)
        require(os.getsid(guard)==guard and os.getpgid(guard)==guard,'guard shares Runtime PTY process group')
        h.delete(reader.name)
        deadline=time.monotonic()+5
        while time.monotonic()<deadline and (mode().split('|')[0]!='0' or mode().split('|')[2]):time.sleep(.03)
        require(mode().split('|')[0]=='0' and not mode().split('|')[2],'Runtime DELETE group kill retained owned mode')
        result['cases']['runtimeDeleteDetachedGuardCleanup']=True
        reader=Reader(h,td,key)
        # SIGKILL cannot rely on Node finally: its parent-held pipe guard must release.
        reader.rpc('scroll',lines=-3);require(mode().startswith('1|'),'kill fixture did not hold')
        pid=int(mode().split('|')[2].split(':')[0]);start=Path(f'/proc/{pid}/stat').read_text().rsplit(') ',1)[1].split()[19]
        require(Path(f'/proc/{pid}/stat').read_text().rsplit(') ',1)[1].split()[19]==start,'helper PID reused')
        os.kill(pid,signal.SIGKILL)
        deadline=time.monotonic()+4
        while time.monotonic()<deadline and mode().split('|')[0]!='0':time.sleep(.03)
        require(mode().split('|')[0]=='0' and not mode().split('|')[2],'SIGKILL pipe guard failed to release only owned mode')
        result['cases']['sigkillPipeGuardCleanup']=True
        # Guard death cannot leave an orphaned owned mode; Node fallback uses exact pane identity/tag.
        reader=Reader(h,td,key);reader.rpc('scroll',lines=-3);guard=guard_pid(reader)
        os.kill(guard,signal.SIGKILL)
        deadline=time.monotonic()+5
        while time.monotonic()<deadline and mode().split('|')[0]!='0':time.sleep(.03)
        require(mode().split('|')[0]=='0' and not mode().split('|')[2],'guard death retained owned mode')
        result['cases']['guardDeathNodeFallbackCleanup']=True
        # HUP executes graceful helper cleanup on the original private pane.
        reader=Reader(h,td,key);reader.rpc('scroll',lines=-3);node=helper_pid(reader)
        os.kill(node,signal.SIGHUP)
        deadline=time.monotonic()+5
        while time.monotonic()<deadline and mode().split('|')[0]!='0':time.sleep(.03)
        require(mode().split('|')[0]=='0' and not mode().split('|')[2],'HUP retained owned mode')
        result['cases']['sighupCleanup']=True
        require(idle_mode().startswith('1|'),'other namespace operations cancelled private idle reader')
        deadline=idle_started+126
        while time.monotonic()<deadline and idle_mode().startswith('1|'):
            # Keep transport-level WebSocket ping/pong alive without renewing helper API idle time.
            for active in [ws,idle_ws,idle.ws]:
                try:
                    op,raw=active.recv(.01)
                    if active is ws and op==2:pty.extend(raw)
                except (socket.timeout,TimeoutError):pass
            time.sleep(.1)
        require(idle_mode()=='0|','120s no-API idle lease retained owned mode')
        result['cases']['idle120sAndOtherNamespaceProtection']=True
        h.delete(idle_work)
        ws.send(2,b'exit\n');deadline=time.monotonic()+3;closed=False
        while time.monotonic()<deadline:
            op,raw=ws.recv(deadline-time.monotonic())
            if op==8:closed=int.from_bytes(raw[:2],'big')==4004;break
        require(closed,'original work exit did not close Runtime binding')
        result['cases']['workExit4004']=True
        result['passed']=True
        print(json.dumps(result,indent=2),flush=True)
    except Exception as error:
        result['error']=str(error);print(json.dumps(result,indent=2),flush=True);raise
    finally:
        try:(output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
        finally:
            try:h.close()
            finally:shutil.rmtree(root)

if __name__=='__main__':main()
