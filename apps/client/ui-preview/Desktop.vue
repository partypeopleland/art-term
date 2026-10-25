<script setup lang="ts">
import App from '../src/desktop/App.vue'
import { createPreviewTransport } from './transport'
import type { HostHistoryRequest } from '../src/shared/terminal/native'
const transport = createPreviewTransport()
const requests: HostHistoryRequest[] = []
const pending = new Map<string, number[]>()
const states = new Map<string, { records: string[]; offset: number; rows: number; cols: number }>()
let count = 0, epoch = 0
const snapshots = new Map<number, string[]>()
function queue(sid: string, text: string) { pending.get(sid)?.push(...new TextEncoder().encode(text)) }
function redraw(sid: string) {
  const s = states.get(sid); if (!s) return
  const end = Math.max(s.rows, s.records.length - s.offset)
  const rows = s.records.slice(Math.max(0, end - s.rows), end)
  queue(sid, '\x1b[H\x1b[2J' + rows.map((text, i) => `\x1b[${i + 1};1H\x1b[38;2;86;180;233m${text}\x1b[0m`).join('') + `\x1b[${s.rows};1H`)
}
const open = transport.open
transport.open = async (target, rows, cols) => {
  const session = await open(target, rows, cols)
  pending.set(session.id, [])
  states.set(session.id, { records: [], offset: 0, rows, cols })
  queue(session.id, '\x1b[?1049h\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?2004h')
  append(1100)
  return session
}
const poll = transport.poll
transport.poll = async sid => {
  const output = await poll(sid)
  output.bytes.push(...(pending.get(sid)?.splice(0) ?? []))
  return output
}
const resize = transport.resize
transport.resize = async (sid, rows, cols) => {
  const s = states.get(sid); if (s) { s.rows = rows; s.cols = cols; redraw(sid) }
  return resize(sid, rows, cols)
}
transport.runtimeHistory = async (sid, request) => {
  requests.push(request)
  const instance = 'a'.repeat(32), s = states.get(sid)
  if (!s) return { kind: 'unsupported' }
  if (request.op === 'scroll' || request.op === 'follow') {
    if (request.op === 'follow') s.offset = 0
    else s.offset = Math.max(0, Math.min(s.records.length - s.rows, s.offset - request.lines))
    redraw(sid)
    return { kind: 'view', instance, op: request.op, held: s.offset > 0, offset: s.offset, available: true }
  }
  if (request.op === 'viewport' || request.op === 'older') {
    if (request.op === 'viewport') snapshots.set(++epoch, [...s.records])
    const selectedEpoch = request.op === 'older' ? request.epoch : epoch
    const saved = snapshots.get(selectedEpoch) ?? []
    const end = request.op === 'older' ? request.before : saved.length
    const start = Math.max(0, end - (request.op === 'older' ? request.limit : s.rows))
    return { kind: 'page', instance, epoch: selectedEpoch, columns: s.cols, rows: s.rows, rowCount: saved.length, start, end, hasOlder: start > 0, records: saved.slice(start, end).map((text, i) => ({ id: start + i + 1, text, styles: [] })) }
  }
  if (request.op === 'close') { s.offset = 0; redraw(sid) }
  return { kind: 'ack', instance, op: request.op }
}
const select = transport.instanceSelect!
transport.instanceSelect = async (...args) => { await new Promise(resolve => setTimeout(resolve, 1200)); return select(...args) }
const write = transport.write
transport.write = async (sid, input, terminalReply) => {
  if (terminalReply) return
  const s = states.get(sid)
  if (s?.offset) { s.offset = 0; redraw(sid) }
  if (input.every(byte => byte === 127 || byte === 8)) queue(sid, '\b \b'.repeat(input.length))
  else await write(sid, input)
}
function append(amount = 25) {
  const lines = Array.from({ length: amount }, () => `ROW-${String(++count).padStart(4, '0')} 桌面終端示範`)
  for (const [sid, s] of states) {
    s.records.push(...lines)
    if (s.offset) s.offset += amount
    else redraw(sid)
  }
}
Object.assign(window, { __desktopPreview: { requests, append } })
</script>
<template><App :transport="transport" /></template>
