/** One common, pipe-owned guard. No platform keys, mouse options, or installed sidecar. */
import { spawn, execFile, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { readFileSync, statSync } from 'node:fs'

const GUARD = String.raw`import base64,ctypes,json,os,select,signal,stat,subprocess,sys,time
cfg=json.loads(sys.stdin.readline())
TAG='@art_term_native_view'
owner=cfg['owner']; pane=None; pane_pid=None; pane_start=None

def proc(pid):
    if os.stat('/proc/'+str(pid)).st_uid!=os.getuid(): raise ValueError()
    f=open('/proc/'+str(pid)+'/stat').read().rsplit(') ',1)[1].split()
    return int(f[1]),f[19]

def identity(worker=True):
    m=cfg['manifest']
    cp,cs=proc(m['clientPid'])
    if cp!=m['runtimePid'] or cs!=m['clientStart'] or proc(m['runtimePid'])[1]!=m['runtimeStart']: raise ValueError()
    if worker:
        wp,ws=proc(cfg['workerPid'])
        if wp!=m['runtimePid'] or ws!=cfg['workerStart'] or os.getppid()!=cfg['workerPid']: raise ValueError()
    for file,mode,kind in [(cfg['state'],0o700,stat.S_ISDIR),(cfg['state']+'/manifest.json',0o600,stat.S_ISREG)]:
        info=os.lstat(file)
        if info.st_uid!=os.getuid() or stat.S_IMODE(info.st_mode)!=mode or not kind(info.st_mode): raise ValueError()
    p=os.path.abspath(cfg['state'])
    while True:
        if stat.S_ISLNK(os.lstat(p).st_mode): raise ValueError()
        q=os.path.dirname(p)
        if q==p: break
        p=q
    if json.load(open(cfg['state']+'/manifest.json'))!=m: raise ValueError()
    if pane_pid and proc(pane_pid)[1]!=pane_start: raise ValueError()

def child_guard():
    parent=os.getppid()
    if ctypes.CDLL(None,use_errno=True).prctl(1,signal.SIGKILL,0,0,0)!=0: os._exit(1)
    if os.getppid()!=parent: os._exit(1)

def tmux(args,text=None,worker=True):
    identity(worker)
    p=subprocess.run([cfg['tmux'],'-N','-S',cfg['state']+'/tmux.sock',*args],input=None if text is None else text.encode(),stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=2,check=True,preexec_fn=child_guard)
    if len(p.stdout)>16384: raise ValueError()
    return p.stdout.decode().strip()

def same(value): return '#{&&:#{==:#{pane_pid},'+str(pane_pid)+'},#{==:#{'+TAG+'},'+value+'}}'
def release(value,worker=True):
    # CAS and cancel are in the same tmux command queue, never a read-then-cancel.
    body='if-shell -F -t '+pane+' "#{==:#{pane_mode},copy-mode}" "send-keys -X -t '+pane+' cancel" ; set-option -pu -t '+pane+' '+TAG
    return tmux(['if-shell','-F','-t',pane,same(value),body],worker=worker)

def status():
    raw=tmux(['display-message','-p','-t',pane,'#{pane_pid}|#{pane_dead}|#{alternate_on}|#{pane_in_mode}|#{pane_mode}|#{scroll_position}|#{'+TAG+'}'])
    p,dead,alt,mode,name,offset,tag=raw.split('|')
    if int(p)!=pane_pid or dead!='0' or alt not in ('0','1') or mode not in ('0','1'): raise ValueError()
    n=int(offset or '0')
    if not 0<=n<=6000: raise ValueError()
    return alt,mode,name,n,tag

class Busy(Exception): pass

def run(req):
    alt,mode,name,offset,tag=status()
    if tag and tag!=owner:
        parts=tag.split(':')
        if len(parts)!=3 or not parts[0].isdigit() or not parts[1].isdigit() or len(parts[2])!=32 or any(c not in '0123456789abcdef' for c in parts[2]): raise Busy()
        try: dead=proc(int(parts[0]))[1]!=parts[1]
        except FileNotFoundError: dead=True
        if dead:
            release(tag);alt,mode,name,offset,tag=status()
        else: raise Busy()
    if req['op']=='scroll':
        lines=req['lines']
        if type(lines)!=int or not -256<=lines<=256: raise ValueError()
        if mode=='1' and (tag!=owner or name!='copy-mode'): raise Busy()
        if lines==0: return {'held':tag==owner and mode=='1' and name=='copy-mode','offset':offset if tag==owner else 0,'available':True}
        if alt=='1' and tag!=owner: return {'held':False,'offset':0,'available':False}
        if mode=='1' and (tag!=owner or name!='copy-mode'): raise Busy()
        if not tag:
            if lines>0: return {'held':False,'offset':0,'available':True}
            condition='#{&&:'+same('')+',#{==:#{pane_in_mode},0}}'
            body='set-option -p -t '+pane+' '+TAG+' '+owner+' ; copy-mode -eH -t '+pane
            if tmux(['if-shell','-F','-t',pane,condition,body+' ; display-message -p claimed'])!='claimed': raise Busy()
        action='scroll-up' if lines<0 else 'scroll-down'
        body='send-keys -X -t '+pane+' -N '+str(abs(lines))+' '+action
        tmux(['if-shell','-F','-t',pane,same(owner),body])
        alt,mode,name,offset,tag=status()
        if tag==owner and (mode=='0' or offset==0): release(owner);return {'held':False,'offset':0,'available':True}
        if tag!=owner or mode!='1' or name!='copy-mode': raise Busy()
        return {'held':True,'offset':offset,'available':True}
    if req['op']=='follow':
        raw=req.get('input')
        data=b''
        if raw is not None:
            if not isinstance(raw,str): raise ValueError()
            data=base64.b64decode(raw,validate=True)
            if len(data)>4096 or base64.b64encode(data).decode()!=raw: raise ValueError()
        if tag!=owner:
            if tag or mode=='1' or raw is not None: raise Busy()
            return {'held':False,'offset':0,'available':True}
        body='if-shell -F -t '+pane+' "#{==:#{pane_mode},copy-mode}" "send-keys -X -t '+pane+' cancel" ; set-option -pu -t '+pane+' '+TAG
        if data: body+=' ; send-keys -H -t '+pane+' '+' '.join(format(b,'02x') for b in data)
        if tmux(['if-shell','-F','-t',pane,same(owner),body+' ; display-message -p followed'])!='followed': raise Busy()
        return {'held':False,'offset':0,'available':True}
    raise ValueError()

try:
    identity()
    found=tmux(['display-message','-p','-t','work:0.0','#{pane_id},#{pane_pid},#{pane_dead}']).split(',')
    if len(found)!=3 or found[0]!='%0' or not found[0][1:].isdigit() or found[2]!='0': raise ValueError()
    pane,pane_pid=found[0],int(found[1]);pane_start=proc(pane_pid)[1]
    print(json.dumps({'ready':True,'pane':pane,'panePid':pane_pid,'paneStart':pane_start}),flush=True)
    last=time.monotonic();buffer=b''
    while time.monotonic()-last<120:
        ready,_,_=select.select([0],[],[],min(1,120-(time.monotonic()-last)))
        if not ready: continue
        chunk=os.read(0,8193)
        if not chunk: break
        buffer+=chunk
        if len(buffer)>8192: raise ValueError()
        while b'\n' in buffer:
            line,buffer=buffer.split(b'\n',1);req=json.loads(line);last=time.monotonic()
            if req['op']=='heartbeat': identity();continue
            if req['op']=='close':
                release(owner)
                print(json.dumps({'closed':True}),flush=True)
                break
            try: reply={'ok':True,**run(req)}
            except Busy: reply={'ok':False,'error':'busy'}
            except Exception: reply={'ok':False,'error':'unavailable'}
            print(json.dumps(reply),flush=True)
        else: continue
        break
finally:
    if pane:
        try: release(owner,worker=False)
        except Exception: pass
`

type Config = { state: string; manifest: unknown; owner: string; workerPid: number; workerStart: string; tmux: string }
export type ViewState = { held: boolean; offset: number; available: boolean }
export class NativeViewport {
  private child: ChildProcessWithoutNullStreams | undefined
  private pending: { resolve: (v: any) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> } | undefined
  private stopped = false
  private buffer = ''
  private pane: { pane: string; panePid: number; paneStart: string } | undefined
  private heartbeat: ReturnType<typeof setInterval> | undefined
  constructor(private config: Config, private fatal: () => void) {}
  async open(): Promise<void> {
    const child = this.child = spawn('python3', ['-u', '-c', GUARD], { stdio: ['pipe','pipe','pipe'], detached: true })
    child.stderr.resume()
    child.on('error', () => this.failed())
    child.on('exit', () => { this.child = undefined; this.failed() })
    child.stdout.on('data', chunk => {
      this.buffer += chunk.toString('utf8')
      if (this.buffer.length > 16384) { this.failed(); return }
      let pos: number
      while ((pos = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0,pos); this.buffer = this.buffer.slice(pos+1)
        const pending = this.pending; this.pending = undefined
        if (!pending) { this.failed(); return }
        clearTimeout(pending.timer)
        try { pending.resolve(JSON.parse(line)) } catch { pending.reject(new Error('unavailable')); this.failed() }
      }
    })
    const reply = await this.exchange(this.config)
    if (reply.ready !== true || !/^%\d+$/.test(reply.pane) || !Number.isSafeInteger(reply.panePid) || !/^\d+$/.test(reply.paneStart)) throw new Error('unavailable')
    this.pane = reply
    this.heartbeat = setInterval(() => { if (this.child && !this.stopped) this.child.stdin.write('{"op":"heartbeat"}\n') },30000)
  }
  private failed(): void {
    if (this.pending) { clearTimeout(this.pending.timer); this.pending.reject(new Error('unavailable')); this.pending = undefined }
    if (!this.stopped) { this.stopped = true; this.fatal() }
  }
  private exchange(value: unknown): Promise<any> {
    if (!this.child || this.stopped || this.pending) return Promise.reject(new Error('unavailable'))
    return new Promise((resolve,reject) => {
      const timer = setTimeout(() => { this.failed() },8000)
      this.pending = { resolve,reject,timer }
      this.child!.stdin.write(JSON.stringify(value)+'\n', error => { if (error) this.failed() })
    })
  }
  async request(value: { op: 'scroll' | 'follow'; lines?: number; input?: string }): Promise<ViewState> {
    const reply = await this.exchange(value)
    if (!reply.ok) throw new Error(reply.error === 'busy' ? 'busy' : 'unavailable')
    if (typeof reply.held !== 'boolean' || typeof reply.available !== 'boolean' || !Number.isSafeInteger(reply.offset) || reply.offset < 0 || reply.offset > 6000) throw new Error('unavailable')
    return { held: reply.held, offset: reply.offset, available: reply.available }
  }
  async close(): Promise<boolean> {
    let confirmed = !this.pane && !this.child
    if (this.child && !this.stopped && !this.pending) {
      try { confirmed = (await this.exchange({op:'close'})).closed === true } catch {}
    }
    this.stopped = true
    if (this.heartbeat) clearInterval(this.heartbeat)
    const child = this.child
    if (child) {
      child.stdin.end()
      await Promise.race([new Promise<void>(resolve => child.once('exit',()=>resolve())),new Promise<void>(resolve=>setTimeout(resolve,3000))])
      if (this.child) child.kill('SIGKILL')
    }
    // Guard death is not permission to retain mode. Exact tag+pane+process fence only.
    const p = this.pane
    if (!p) return confirmed
    try {
      const folder = `/proc/${p.panePid}`
      if (statSync(folder).uid !== process.getuid!() || readFileSync(folder+'/stat','utf8').split(/\) /).at(-1)!.split(' ')[19] !== p.paneStart) return confirmed
      const condition = `#{&&:#{==:#{pane_pid},${p.panePid}},#{==:#{@art_term_native_view},${this.config.owner}}}`
      const commands = `if-shell -F -t ${p.pane} "#{==:#{pane_mode},copy-mode}" "send-keys -X -t ${p.pane} cancel" ; set-option -pu -t ${p.pane} @art_term_native_view`
      await new Promise<void>(resolve => execFile(this.config.tmux,['-N','-S',this.config.state+'/tmux.sock','if-shell','-F','-t',p.pane,condition,commands],{timeout:2000,maxBuffer:16384},()=>resolve()))
      const tag = await new Promise<string>((resolve,reject) => execFile(this.config.tmux,['-N','-S',this.config.state+'/tmux.sock','display-message','-p','-t',p.pane,'#{@art_term_native_view}'],{timeout:2000,maxBuffer:16384},(error,stdout)=>error?reject(error):resolve(stdout.trim())))
      confirmed = tag !== this.config.owner
    } catch {}
    return confirmed
  }
}
