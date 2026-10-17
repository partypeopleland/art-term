/** Page-only Node 22 worker. tmux owns rendering, history and reader snapshots. */
import { randomBytes } from 'node:crypto'
import { lstatSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'
import { execFile } from 'node:child_process'
import { once } from 'node:events'
import { readStyledLines, type NativeTerminalLine } from '../../../apps/desktop/src/terminal/native-buffer'
import { Terminal } from '../../../apps/desktop/node_modules/@xterm/xterm/lib/xterm.js'

const PAGE_BYTES = 256 * 1024, CAPTURE_BYTES = 4 * 1024 * 1024
const LEASE_MS = 120000, CAPTURE_MS = 500
const key = process.argv[2] ?? '', nonce = process.argv[3] ?? ''
const instance = randomBytes(16).toString('hex')
const state = path.join(homedir(), '.local/share/art-term/tmux-history-v2', key)
class Failure extends Error { constructor(readonly kind: string) { super(kind) } }
const fail = (kind: string): never => { throw new Failure(kind) }
function proc(pid: number): { parent: number; start: string } {
  const folder = `/proc/${pid}`
  if (lstatSync(folder).uid !== process.getuid!()) fail('unavailable')
  const fields = readFileSync(`${folder}/stat`, 'utf8').split(/\) /).at(-1)!.split(' ')
  return { parent: Number(fields[1]), start: fields[19] }
}
let manifest: { v: number; key: string; clientPid: number; clientStart: string; runtimePid: number; runtimeStart: string }
function privatePath(file: string, directory: boolean): void {
  const info = lstatSync(file)
  if (info.uid !== process.getuid!() || (info.mode & 0o777) !== (directory ? 0o700 : 0o600)
      || !(directory ? info.isDirectory() : info.isFile())) fail('unavailable')
}
function identity(initial = false): void {
  if (initial) {
    if (!/^[0-9a-f]{24}$/.test(key) || !/^[0-9a-f]{32}$/.test(nonce)) fail('protocol')
    for (let parent = state; ; parent = path.dirname(parent)) {
      if (lstatSync(parent).isSymbolicLink()) fail('unavailable')
      if (parent === path.dirname(parent)) break
    }
    privatePath(state, true)
    privatePath(path.join(state, 'manifest.json'), false)
    const raw = readFileSync(path.join(state, 'manifest.json'))
    if (raw.length > 4096) fail('unavailable')
    manifest = JSON.parse(raw.toString('utf8'))
    if (manifest.v !== 2 || manifest.key !== key || !Number.isSafeInteger(manifest.clientPid)
        || manifest.clientPid < 2 || !Number.isSafeInteger(manifest.runtimePid) || manifest.runtimePid < 2
        || !/^\d+$/.test(manifest.clientStart) || !/^\d+$/.test(manifest.runtimeStart)) fail('unavailable')
  }
  const client = proc(manifest.clientPid), runtime = proc(manifest.runtimePid)
  if (client.start !== manifest.clientStart || client.parent !== manifest.runtimePid
      || runtime.start !== manifest.runtimeStart || process.ppid !== manifest.runtimePid
      || proc(process.ppid).start !== manifest.runtimeStart) fail('unavailable')
}
function tmux(args: string[], input?: string): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const child = execFile(path.join(homedir(), '.local/share/art-term/openab-pty/tmux-v2/tmux'), ['-S', path.join(state, 'tmux.sock'), ...args],
      { timeout: 2000, maxBuffer: CAPTURE_BYTES, encoding: 'buffer' },
      (error, stdout) => error ? reject(new Failure((error as any).code === 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER' ? 'limit' : 'unavailable')) : resolve(stdout))
    child.stdin?.on('error', () => {})
    child.stdin?.end(input)
  })
}
// tmux owns the full backing grids. This worker stores only reader metadata.
type Snapshot = { epoch: number; columns: number; rows: number; pane: string; window: string; history: number; rowCount: number; expires: number; published: boolean }
const snapshots = new Map<number, Snapshot>()
const owned = new Set<string>()
const owner = `${process.pid}:${proc(process.pid).start}:${instance}`
let displayed: number | undefined, inflight: number | undefined, sequence = 0, lastCapture = 0
const TAG = '@art_term_reader'
async function removeWindow(window: string): Promise<void> {
  if (!owned.has(window)) return
  const tag = (await tmux(['display-message', '-p', '-t', window, `#{${TAG}}`])).toString().trim()
  if (tag !== owner) fail('unavailable')
  await tmux(['kill-window', '-t', window])
  owned.delete(window)
}
async function release(epoch: number): Promise<void> {
  const snapshot = snapshots.get(epoch)
  if (snapshot) await removeWindow(snapshot.window)
  snapshots.delete(epoch)
  if (displayed === epoch) displayed = undefined
  if (inflight === epoch) inflight = undefined
}
async function prune(): Promise<void> {
  for (const [epoch, value] of snapshots) if (value.expires <= Date.now()) await release(epoch)
}
async function cleanup(): Promise<void> {
  await restoreTerminalMouse().catch(() => {})
  await removeBuffer(instance).catch(() => {})
  for (const window of [...owned]) await removeWindow(window).catch(() => {})
}
async function cleanStale(): Promise<void> {
  const raw = (await tmux(['list-windows', '-t', 'work', '-F', `#{window_id},#{${TAG}}`])).toString()
  for (const line of raw.trim().split('\n')) {
    const match = /^(@\d+),(\d+):(\d+):([0-9a-f]{32})$/.exec(line)
    if (!match) continue
    let dead = false
    try { dead = proc(Number(match[2])).start !== match[3] }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') dead = true; else throw error }
    if (!dead) continue
    // Recheck the exact dead-owner tag in the server queue before deletion.
    const tag = `${match[2]}:${match[3]}:${match[4]}`
    await tmux(['if-shell', '-F', '-t', match[1], `#{==:#{${TAG}},${tag}}`, `kill-window -t ${match[1]}`])
    await removeBuffer(match[4])
  }
}
function get(epoch: unknown): Snapshot {
  if (!Number.isSafeInteger(epoch) || Number(epoch) < 1) fail('protocol')
  const snapshot = snapshots.get(Number(epoch))
  if (!snapshot || snapshot.expires <= Date.now()) fail('expired')
  return snapshot
}
function renew(snapshot: Snapshot): void { snapshot.expires = Date.now() + LEASE_MS }
const SIZE = ['display-message', '-p', '-t', 'work:0.0', '#{pane_width},#{pane_height},#{pane_dead},#{alternate_on}']
function parseDimensions(raw: string, allowAlternate = false): { columns: number; rows: number } {
  const match = /^(\d+),(\d+),0,([01])$/.exec(raw)
  if (!match) fail('unavailable')
  if (match[3] === '1' && !allowAlternate) fail('unsupported')
  const columns = Number(match[1]), rows = Number(match[2])
  if (columns < 1 || columns > 500 || rows < 1 || rows > 500) fail('limit')
  return { columns, rows }
}
async function dimensions(): Promise<{ columns: number; rows: number }> {
  return parseDimensions((await tmux(SIZE)).toString('utf8').trim())
}
// The lock covers original capture, owner handoff, mutation and restoration.
// Python and tmux children have bounded lifetimes; no persistent lock owner.
const MOUSE_TRANSACTION = String.raw`import base64, ctypes, fcntl, json, os, signal, stat, subprocess, sys, time
class Busy(Exception): pass
cfg=json.load(sys.stdin)
transaction_pid=os.getpid()
started=time.monotonic(); deadline=started+6
STATE='@art_term_native_mouse_state'
lock=None

def process(pid):
    folder='/proc/'+str(pid)
    if os.stat(folder).st_uid != os.getuid(): raise ValueError()
    fields=open(folder+'/stat').read().rsplit(') ',1)[1].split()
    return int(fields[1]),fields[19]

def identity():
    parent,start=process(cfg['workerPid'])
    if os.getppid()!=cfg['workerPid'] or start!=cfg['workerStart']: raise ValueError()
    m=cfg['manifest']
    if parent!=m['runtimePid'] or process(parent)[1]!=m['runtimeStart']: raise ValueError()
    cp,cs=process(m['clientPid'])
    if cp!=m['runtimePid'] or cs!=m['clientStart']: raise ValueError()
    for p,mode,kind in [(cfg['state'],0o700,stat.S_ISDIR),(os.path.join(cfg['state'],'manifest.json'),0o600,stat.S_ISREG)]:
        info=os.lstat(p)
        if info.st_uid!=os.getuid() or stat.S_IMODE(info.st_mode)!=mode or not kind(info.st_mode): raise ValueError()
    p=os.path.abspath(cfg['state'])
    while True:
        if stat.S_ISLNK(os.lstat(p).st_mode): raise ValueError()
        q=os.path.dirname(p)
        if q==p: break
        p=q
    if json.load(open(os.path.join(cfg['state'],'manifest.json'))) != m: raise ValueError()

def death_guard():
    # A killed transaction must not leave a tmux client writing after it dies.
    if ctypes.CDLL(None,use_errno=True).prctl(1,signal.SIGKILL,0,0,0)!=0: os._exit(1)
    if os.getppid()!=transaction_pid: os.kill(os.getpid(),signal.SIGKILL)

def tmux(args,text=None):
    remaining=deadline-time.monotonic()
    if remaining<=0: raise Busy()
    identity()
    done=subprocess.run([cfg['tmux'],'-N','-S',os.path.join(cfg['state'],'tmux.sock'),*args],
        input=None if text is None else text.encode(),stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,
        timeout=min(2,remaining),check=True,close_fds=True,preexec_fn=death_guard)
    if len(done.stdout)>16384: raise ValueError()
    return done.stdout.decode().strip()

def equal(value): return '#{==:#{'+STATE+'},'+value+'}'
def wheel(value):
    if not value.startswith('bind-key ') or '\n' in value or '\r' in value or len(value)>4096: raise ValueError()
    return value

def decode(value):
    if len(value)>8192: raise ValueError()
    state=json.loads(base64.b64decode(value,validate=True))
    if set(state)!={'v','owner','initial'} or state['v']!=1: raise ValueError()
    if len(state['owner'])!=32 or any(c not in '0123456789abcdef' for c in state['owner']): raise ValueError()
    initial=state['initial']
    if set(initial)!={'mouse','copy','wheel'} or initial['mouse'] not in ('on','off') or initial['copy'] not in ('0','1'): raise ValueError()
    wheel(initial['wheel'])
    return state

try:
    identity()
    lock=os.open(os.path.join(cfg['state'],'native-mouse.lock'),os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW|os.O_CLOEXEC,0o600)
    info=os.fstat(lock)
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=os.getuid() or stat.S_IMODE(info.st_mode)!=0o600: raise ValueError()
    until=min(deadline,time.monotonic()+2)
    while True:
        try: fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB); break
        except BlockingIOError:
            if time.monotonic()>=until: raise Busy()
            time.sleep(.01)
    identity()
    previous=tmux(['show-options','-gqv',STATE])
    current=decode(previous) if previous else None
    if cfg['enabled']:
        if current: initial=current['initial']
        else:
            initial={'mouse':tmux(['show-options','-gqv','mouse']),
                     'copy':tmux(['display-message','-p','-t','work:0.0','#{pane_in_mode}']),
                     'wheel':wheel(tmux(['list-keys','-T','root','WheelUpPane']))}
            if initial['mouse'] not in ('on','off') or initial['copy'] not in ('0','1'): raise ValueError()
        encoded=base64.b64encode(json.dumps({'v':1,'owner':cfg['instance'],'initial':initial},separators=(',',':')).encode()).decode()
        # Complete original state is committed before any mouse/binding mutation.
        staged=tmux(['if-shell','-F',equal(previous),'set-option -g '+STATE+' '+encoded+'; display-message -p staged'])
        if staged!='staged': raise Busy()
        script='set-option -g mouse on\nbind-key -T root WheelUpPane if-shell -F -t = "#{||:#{pane_in_mode},#{mouse_any_flag}}" "send-keys -M" "copy-mode -e -t =; send-keys -X -t = -N 2 scroll-up"\ndisplay-message -p enabled\n'
        if tmux(['if-shell','-F',equal(encoded),'source-file -'],script)!='enabled': raise Busy()
    elif current and current['owner']==cfg['instance']:
        initial=current['initial']
        script=initial['wheel']+'\n'
        if initial['copy']=='0': script+='if-shell -F -t work:0.0 "#{==:#{pane_mode},copy-mode}" "send-keys -X -t work:0.0 cancel"\n'
        script+='set-option -g mouse '+initial['mouse']+'\nset-option -gu '+STATE+'\ndisplay-message -p restored\n'
        # Final unset follows exact restoration, fenced against a successor.
        tmux(['if-shell','-F',equal(previous),'source-file -'],script)
    print('ok')
except (Busy,subprocess.TimeoutExpired):
    print('busy'); sys.exit(2)
except Exception:
    print('unavailable'); sys.exit(1)
finally:
    if lock is not None: os.close(lock)
`
function terminalMouse(enabled: boolean): Promise<void> {
  identity()
  return new Promise((resolve, reject) => {
    const child = execFile('python3', ['-c', MOUSE_TRANSACTION],
      { timeout: 8000, maxBuffer: 16384, encoding: 'utf8' },
      (error, stdout) => error ? reject(new Failure(stdout.trim() === 'busy' ? 'busy' : 'unavailable')) : resolve())
    child.stdin?.on('error', () => {})
    child.stdin?.end(JSON.stringify({ state, manifest, instance, enabled,
      workerPid: process.pid, workerStart: proc(process.pid).start,
      tmux: path.join(homedir(), '.local/share/art-term/openab-pty/tmux-v2/tmux') }))
  })
}
async function restoreTerminalMouse(): Promise<void> { await terminalMouse(false) }
function takeLine(raw: Buffer): [string, Buffer] {
  const newline = raw.indexOf(10)
  if (newline < 0 || newline > 100) fail('protocol')
  return [raw.subarray(0, newline).toString(), raw.subarray(newline + 1)]
}
function mode(raw: string, columns: number, rows: number): number {
  const match = /^(\d+),1,copy-mode,(\d+),(\d+)$/.exec(raw)
  if (!match || Number(match[2]) !== columns || Number(match[3]) !== rows) fail('unavailable')
  const history = Number(match[1])
  if (history > 6000) fail('limit')
  return history
}
const READER = '#{scroll_position},#{pane_in_mode},#{pane_mode},#{pane_width},#{pane_height}'
async function decode(raw: Buffer, columns: number, count: number, firstId: number, cursor = count - 1): Promise<NativeTerminalLine[]> {
  const term = new Terminal({ cols: columns, rows: count + 1, scrollback: 0, allowProposedApi: true })
  try {
    await new Promise<void>(resolve => term.write(raw.toString('utf8').replace(/\n$/, '').replace(/\n/g, '\r\n'), resolve))
    if (term.buffer.active.baseY !== 0 || term.buffer.active.cursorY !== count - 1) fail('unsupported')
    await new Promise<void>(resolve => term.write(`\x1b[${Math.max(0, Math.min(cursor, count - 1)) + 1};1H`, resolve))
    const projection = readStyledLines(term.buffer.active, firstId)
    if (!projection.supported || projection.records.length > count) fail('limit')
    return projection.records
  } finally { term.dispose() }
}
async function removeBuffer(id: string): Promise<void> {
  const name = `at-row-${id}`
  if ((await tmux(['list-buffers', '-f', `#{==:#{buffer_name},${name}}`, '-F', '#{buffer_size}'])).length)
    await tmux(['delete-buffer', '-b', name])
}
async function readRows(snapshot: Snapshot, start: number, end: number, cursor?: number): Promise<NativeTerminalLine[]> {
  const first = Math.max(0, start - 1), count = end - first, buffer = `at-row-${instance}`
  const commands: string[][] = [['display-message', '-p', '-t', snapshot.pane, READER]]
  for (let row = first; row < end; row++) {
    const options = ['-M', '-e', '-J', '-N', '-S', String(row - snapshot.history), '-E', String(row - snapshot.history), '-t', snapshot.pane]
    commands.push(['set-buffer', '-b', buffer, 'xx'], ['capture-pane', ...options, '-b', buffer],
      ['list-buffers', '-f', `#{==:#{buffer_name},${buffer}}`, '-F', '#{buffer_size}'], ['capture-pane', '-p', ...options])
  }
  commands.push(['delete-buffer', '-b', buffer], ['display-message', '-p', '-t', snapshot.pane, READER])
  // All tokens are static or validated native IDs/numbers; never include CLI text.
  const script = commands.map(command => command.map(token => {
    if (/[\'\r\n\0]/.test(token)) fail('protocol')
    return `'${token}'`
  }).join(' ')).join('\n') + '\n'
  if (Buffer.byteLength(script) > 512 * 1024) fail('limit')
  const framed = await tmux(['source-file', '-'], script).catch(async error => {
    await removeBuffer(instance).catch(() => {})
    throw error
  })
  let [metadata, raw] = takeLine(framed)
  if (mode(metadata, snapshot.columns, snapshot.rows) !== snapshot.history) fail('unavailable')
  const text: string[] = [], wraps: boolean[] = []
  for (let i = 0; i < count; i++) {
    let size: string
    ;[size, raw] = takeLine(raw)
    if (!/^\d+$/.test(size)) fail('protocol')
    const newline = raw.indexOf(10)
    if (newline < 0 || newline >= PAGE_BYTES) fail('limit')
    let nativeSize = Number(size)
    const wireSize = newline + 1
    if (nativeSize === 2 && wireSize === 1) nativeSize = 0
    if (nativeSize !== wireSize && nativeSize !== wireSize - 1) fail('protocol')
    wraps.push(nativeSize === wireSize - 1)
    text.push('\x1b[0m' + raw.subarray(0, newline).toString('utf8'))
    raw = raw.subarray(newline + 1)
  }
  ;[metadata, raw] = takeLine(raw)
  if (raw.length || mode(metadata, snapshot.columns, snapshot.rows) !== snapshot.history) fail('unavailable')
  const records = await decode(Buffer.from(text.join('\n')), snapshot.columns + 1, count, first + 1,
    cursor === undefined ? count - 1 : cursor + start - first)
  for (let i = 0; i < records.length; i++) records[i].wrapped = i > 0 && wraps[i - 1]
  return records.slice(start - first)
}
async function capture(): Promise<Snapshot> {
  await prune()
  if (inflight !== undefined) return get(inflight)
  if (Date.now() - lastCapture < CAPTURE_MS && displayed !== undefined) return get(displayed)
  for (let attempt = 0; attempt < 2; attempt++) {
    identity()
    const expected = await dimensions()
    const epoch = ++sequence, name = `at-${instance}-${epoch}`
    const created = (await tmux(['new-window', '-d', '-P', '-F', '#{window_id},#{pane_id}', '-n', name, '-t', 'work', 'sleep 2147483647',
      ';', 'set-window-option', '-t', `work:${name}`, 'automatic-rename', 'off',
      ';', 'set-window-option', '-t', `work:${name}`, TAG, owner])).toString().trim()
    const match = /^(@\d+),(%\d+)$/.exec(created)
    if (!match) fail('unavailable')
    const window = match[1], pane = match[2]
    owned.add(window)
    try {
      await tmux(['set-window-option', '-t', window, 'window-size', 'manual', ';', 'resize-window', '-t', window, '-x', String(expected.columns), '-y', String(expected.rows)])
      const captured = await tmux([...SIZE, ';', 'copy-mode', '-H', '-s', 'work:0.0', '-t', pane,
        ';', 'display-message', '-p', '-t', pane, '#{copy_cursor_y},#{scroll_position}',
        ';', 'send-keys', '-X', '-t', pane, 'history-top',
        ';', 'display-message', '-p', '-t', pane, READER])
      let [source, rest] = takeLine(captured)
      const size = parseDimensions(source)
      let cursor: string, metadata: string
      ;[cursor, rest] = takeLine(rest); [metadata, rest] = takeLine(rest)
      if (size.columns !== expected.columns || size.rows !== expected.rows) { await removeWindow(window); continue }
      const history = mode(metadata, size.columns, size.rows)
      const cursorMatch = /^(\d+),(\d+)$/.exec(cursor)
      if (!cursorMatch) fail('protocol')
      if (rest.length) fail('protocol')
      const snapshot: Snapshot = { epoch, ...size, pane, window, history, rowCount: history + size.rows, expires: Date.now() + LEASE_MS, published: false }
      const records = await readRows(snapshot, history, history + size.rows, Number(cursorMatch[1]) - Number(cursorMatch[2]))
      const rowCount = history + records.length
      if (rowCount < history + 1 || rowCount > 6000) fail('limit')
      snapshot.rowCount = rowCount
      const after = await dimensions()
      if (after.columns !== size.columns || after.rows !== size.rows) { await removeWindow(window); continue }
      identity()
      snapshots.set(epoch, snapshot); inflight = epoch; lastCapture = Date.now()
      return snapshot
    } catch (error) { await removeWindow(window); throw error }
  }
  return fail('busy')
}
async function send(requestId: number, fields: Record<string, unknown>): Promise<void> {
  const raw = JSON.stringify({ v: 1, nonce, requestId, instance, ...fields }) + '\n'
  if (Buffer.byteLength(raw) > PAGE_BYTES) fail('limit')
  if (!process.stdout.write(raw)) await once(process.stdout, 'drain')
}
async function page(requestId: number, op: string, snapshot: Snapshot, start: number, end: number): Promise<void> {
  const records = await readRows(snapshot, start, end)
  if (records.length !== end - start) fail('unsupported')
  await send(requestId, { type: 'page', op, epoch: snapshot.epoch, columns: snapshot.columns, rows: snapshot.rows,
    rowCount: snapshot.rowCount, start, end, hasOlder: start > 0, records })
  snapshot.published = true
  renew(snapshot)
}
let lastRequest = 0, closing = false
let idleTimer: ReturnType<typeof setTimeout> | undefined
function armIdle(): void {
  clearTimeout(idleTimer)
  if (!closing) idleTimer = setTimeout(() => {
    closing = true
    process.stdin.destroy()
  }, LEASE_MS)
}
async function request(value: any): Promise<void> {
  let requestId = Number.isSafeInteger(value?.requestId) && value.requestId > 0 ? value.requestId : 0
  let authenticated = false
  try {
    if (!value || value.v !== 1 || value.nonce !== nonce || requestId <= lastRequest
      || !['viewport', 'older', 'ack', 'keep', 'close', 'terminalMouse'].includes(value.op)) fail('protocol')
    lastRequest = requestId
    authenticated = true
    clearTimeout(idleTimer)
    identity()
    await prune()
    if (value.op === 'terminalMouse') {
      if (typeof value.enabled !== 'boolean') fail('protocol')
      await terminalMouse(value.enabled)
      await send(requestId, { type: 'ack', op: value.op })
    } else if (value.op === 'viewport') {
      const snapshot = await capture()
      await page(requestId, value.op, snapshot, snapshot.history, snapshot.rowCount)
    } else if (value.op === 'older') {
      const snapshot = get(value.epoch)
      if (!Number.isSafeInteger(value.before) || value.before <= 0 || value.before > snapshot.rowCount
        || !Number.isSafeInteger(value.limit) || value.limit < 1 || value.limit > 256) fail('protocol')
      await page(requestId, value.op, snapshot, Math.max(0, value.before - value.limit), value.before)
    } else if (value.op === 'ack' || value.op === 'keep') {
      const snapshot = get(value.epoch)
      if (value.op === 'ack') {
        if (snapshot.epoch !== displayed && snapshot.epoch !== inflight) fail('protocol')
        if (snapshot.epoch === inflight) {
          if (displayed !== undefined) await release(displayed)
          displayed = inflight; inflight = undefined
        }
      }
      renew(snapshot)
      await send(requestId, { type: 'ack', op: value.op, epoch: snapshot.epoch })
    } else {
      await send(requestId, { type: 'ack', op: 'close' })
      closing = true
    }
  } catch (error) {
    // A page rejected before transmission is not an offered candidate. Keep
    // already-published candidates pinned, but allow a later explicit retry
    // after an oversized viewport becomes small enough to transmit.
    if (inflight !== undefined && snapshots.get(inflight)?.published === false) {
      await release(inflight)
    }
    await send(requestId, { type: 'error', kind: error instanceof Failure ? error.kind : 'unavailable' })
  } finally {
    if (authenticated) armIdle()
  }
}
async function main(): Promise<void> {
  identity(true)
  parseDimensions((await tmux(SIZE)).toString('utf8').trim(), true)
  await cleanStale()
  if (process.stdin.isTTY) process.stdin.setRawMode(true)
  // Interactive bash emits its final bracketed-paste control before exec.
  // Put that pre-ready noise on its own line; never require a VT parser here.
  process.stdout.write('\n')
  await send(0, { type: 'ready', key, terminalMouse: true })
  armIdle()
  // Async iteration pauses reading during each request; no unbounded application queue.
  let buffer = Buffer.alloc(0)
  for await (const chunk of process.stdin) {
    buffer = Buffer.concat([buffer, chunk])
    if (buffer.length > 8192) fail('protocol')
    let newline: number
    while ((newline = buffer.indexOf(10)) !== -1) {
      const line = buffer.subarray(0, newline).toString('utf8').replace(/\r$/, '')
      buffer = buffer.subarray(newline + 1)
      try { await request(JSON.parse(line)) } catch { await send(0, { type: 'error', kind: 'protocol' }) }
      if (closing) return
    }
  }
}
for (const signal of ['SIGTERM', 'SIGINT'] as const) process.on(signal, () => { closing = true; process.stdin.destroy() })
main().catch(async error => {
  if (closing) return
  try { await send(0, { type: 'error', kind: error instanceof Failure ? error.kind : 'unavailable' }) } finally { process.exitCode = 1 }
})

.finally(async () => { clearTimeout(idleTimer); await cleanup() })
