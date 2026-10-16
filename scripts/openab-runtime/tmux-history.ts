/** Private Node 22 history worker. Work PTY remains plain tmux VT. */
import { randomBytes } from 'node:crypto'
import { lstatSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'
import { execFile } from 'node:child_process'
import { once } from 'node:events'
import { readStyledLines, type NativeTerminalLine } from '../../apps/desktop/src/terminal/native-buffer'
import { Terminal } from '../../apps/desktop/node_modules/@xterm/xterm/lib/xterm.js'

const PAGE_BYTES = 256 * 1024, SNAPSHOT_BYTES = 4 * 1024 * 1024
const LEASE_MS = 120000, CAPTURE_MS = 500
const key = process.argv[2] ?? '', nonce = process.argv[3] ?? ''
const instance = randomBytes(16).toString('hex')
const state = path.join(homedir(), '.local/share/art-term/tmux-history', key)
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
    if (manifest.v !== 1 || manifest.key !== key || !Number.isSafeInteger(manifest.clientPid)
        || manifest.clientPid < 2 || !Number.isSafeInteger(manifest.runtimePid) || manifest.runtimePid < 2
        || !/^\d+$/.test(manifest.clientStart) || !/^\d+$/.test(manifest.runtimeStart)) fail('unavailable')
  }
  const client = proc(manifest.clientPid), runtime = proc(manifest.runtimePid)
  if (client.start !== manifest.clientStart || client.parent !== manifest.runtimePid
      || runtime.start !== manifest.runtimeStart || process.ppid !== manifest.runtimePid
      || proc(process.ppid).start !== manifest.runtimeStart) fail('unavailable')
}
function tmux(args: string[]): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    execFile('tmux', ['-S', path.join(state, 'tmux.sock'), ...args],
      { timeout: 2000, maxBuffer: SNAPSHOT_BYTES, encoding: 'buffer' },
      (error, stdout) => error ? reject(new Failure((error as any).code === 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER' ? 'limit' : 'unavailable')) : resolve(stdout))
  })
}
type Snapshot = { epoch: number; columns: number; rows: number; records: NativeTerminalLine[]; raw: Buffer; expires: number; published: boolean }
const snapshots = new Map<number, Snapshot>()
let displayed: number | undefined, inflight: number | undefined, sequence = 0, lastCapture = 0
function get(epoch: unknown): Snapshot {
  if (!Number.isSafeInteger(epoch) || Number(epoch) < 1) fail('protocol')
  const snapshot = snapshots.get(Number(epoch))
  if (!snapshot || snapshot.expires <= Date.now()) fail('expired')
  return snapshot
}
function renew(snapshot: Snapshot): void { snapshot.expires = Date.now() + LEASE_MS }
const SIZE = ['display-message', '-p', '-t', 'work:0.0', '#{pane_width},#{pane_height},#{pane_dead},#{alternate_on}']
function parseDimensions(raw: string): { columns: number; rows: number } {
  const match = /^(\d+),(\d+),0,([01])$/.exec(raw)
  if (!match) fail('unavailable')
  if (match[3] === '1') fail('unsupported')
  const columns = Number(match[1]), rows = Number(match[2])
  if (columns < 1 || columns > 500 || rows < 1 || rows > 500) fail('limit')
  return { columns, rows }
}
async function dimensions(): Promise<{ columns: number; rows: number }> {
  return parseDimensions((await tmux(SIZE)).toString('utf8').trim())
}
async function capture(): Promise<Snapshot> {
  // Only an explicit viewport request may leave an expired epoch for a fresh one.
  for (const [epoch, value] of snapshots) if (value.expires <= Date.now()) {
    snapshots.delete(epoch)
    if (displayed === epoch) displayed = undefined
    if (inflight === epoch) inflight = undefined
  }
  if (inflight !== undefined) return get(inflight)
  if (Date.now() - lastCapture < CAPTURE_MS && displayed !== undefined) return get(displayed)
  identity()
  let raw: Buffer | undefined, size: { columns: number; rows: number } | undefined
  for (let attempt = 0; attempt < 2; attempt++) {
    // Both commands run synchronously in the same tmux command queue.
    // Independent pre/post queries cannot rule out a resize ABA race.
    const captured = await tmux([...SIZE, ';', 'capture-pane', '-p', '-e', '-J', '-S', '-', '-t', 'work:0.0'])
    const newline = captured.indexOf(10)
    if (newline < 0 || newline > 80) fail('protocol')
    size = parseDimensions(captured.subarray(0, newline).toString('utf8'))
    raw = captured.subarray(newline + 1)
    const after = await dimensions()
    if (size.columns === after.columns && size.rows === after.rows) break
    raw = undefined
  }
  if (!raw || !size) fail('busy')
  identity()
  lastCapture = Date.now()
  if (displayed !== undefined) {
    const previous = get(displayed)
    if (raw.equals(previous.raw) && previous.columns === size.columns && previous.rows === size.rows) {
      renew(previous); return previous
    }
  }
  const term = new Terminal({ cols: size.columns, rows: size.rows, scrollback: 5000, allowProposedApi: true })
  let records: NativeTerminalLine[]
  try {
    await new Promise<void>(resolve => term.write(raw!.toString('utf8').replace(/\n$/, '').replace(/\n/g, '\r\n'), resolve))
    const projection = readStyledLines(term.buffer.active)
    if (!projection.supported) fail('limit')
    records = projection.records
  } finally { term.dispose() }
  identity()
  if (!records!.length || records!.length > 6000 || Buffer.byteLength(JSON.stringify(records!)) > SNAPSHOT_BYTES) fail('limit')
  const snapshot: Snapshot = { epoch: ++sequence, ...size, records: records!, raw, expires: Date.now() + LEASE_MS, published: false }
  snapshots.set(snapshot.epoch, snapshot)
  inflight = snapshot.epoch
  return snapshot
}
async function send(requestId: number, fields: Record<string, unknown>): Promise<void> {
  const raw = JSON.stringify({ v: 1, nonce, requestId, instance, ...fields }) + '\n'
  if (Buffer.byteLength(raw) > PAGE_BYTES) fail('limit')
  if (!process.stdout.write(raw)) await once(process.stdout, 'drain')
}
async function page(requestId: number, op: string, snapshot: Snapshot, start: number, end: number): Promise<void> {
  await send(requestId, { type: 'page', op, epoch: snapshot.epoch, columns: snapshot.columns, rows: snapshot.rows,
    rowCount: snapshot.records.length, start, end, hasOlder: start > 0, records: snapshot.records.slice(start, end) })
  snapshot.published = true
  renew(snapshot)
}
let lastRequest = 0, closing = false
async function request(value: any): Promise<void> {
  let requestId = Number.isSafeInteger(value?.requestId) && value.requestId > 0 ? value.requestId : 0
  try {
    if (!value || value.v !== 1 || value.nonce !== nonce || requestId <= lastRequest
      || !['viewport', 'older', 'ack', 'keep', 'close'].includes(value.op)) fail('protocol')
    lastRequest = requestId
    identity()
    if (value.op === 'viewport') {
      const snapshot = await capture()
      await page(requestId, value.op, snapshot, Math.max(0, snapshot.records.length - snapshot.rows), snapshot.records.length)
    } else if (value.op === 'older') {
      const snapshot = get(value.epoch)
      if (!Number.isSafeInteger(value.before) || value.before <= 0 || value.before > snapshot.records.length
        || !Number.isSafeInteger(value.limit) || value.limit < 1 || value.limit > 256) fail('protocol')
      await page(requestId, value.op, snapshot, Math.max(0, value.before - value.limit), value.before)
    } else if (value.op === 'ack' || value.op === 'keep') {
      const snapshot = get(value.epoch)
      if (value.op === 'ack') {
        if (snapshot.epoch !== displayed && snapshot.epoch !== inflight) fail('protocol')
        if (snapshot.epoch === inflight) {
          if (displayed !== undefined) snapshots.delete(displayed)
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
      snapshots.delete(inflight)
      inflight = undefined
    }
    await send(requestId, { type: 'error', kind: error instanceof Failure ? error.kind : 'unavailable' })
  }
}
async function main(): Promise<void> {
  identity(true)
  await dimensions()
  if (process.stdin.isTTY) process.stdin.setRawMode(true)
  // Interactive bash emits its final bracketed-paste control before exec.
  // Put that pre-ready noise on its own line; never require a VT parser here.
  process.stdout.write('\n')
  await send(0, { type: 'ready', key })
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
main().catch(async error => {
  try { await send(0, { type: 'error', kind: error instanceof Failure ? error.kind : 'unavailable' }) } finally { process.exitCode = 1 }
})
