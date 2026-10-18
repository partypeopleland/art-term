<script setup lang="ts">
import App from '../src/App.vue'
import { createPreviewTransport } from './transport'
import type { HostHistoryRequest } from '../src/terminal/native'

const transport = createPreviewTransport()
const requests: HostHistoryRequest[] = []
const pending = new Map<string, number[]>()
let count = 0
let epoch = 0
const snapshots = new Map<number, string[]>()
const records: string[] = []
const open = transport.open
transport.open = async (...args) => {
  const session = await open(...args)
  pending.set(session.id, [])
  append(96)
  return session
}
const poll = transport.poll
transport.poll = async sid => {
  const output = await poll(sid)
  output.bytes.push(...(pending.get(sid)?.splice(0) ?? []))
  return output
}
transport.runtimeHistory = async (_sid, request) => {
  requests.push(request)
  const instance = 'a'.repeat(32)
  if (request.op === 'viewport' || request.op === 'older') {
    if (request.op === 'viewport') snapshots.set(++epoch, [...records])
    const selectedEpoch = request.op === 'older' ? request.epoch : epoch
    const saved = snapshots.get(selectedEpoch) ?? []
    const end = request.op === 'older' ? request.before : saved.length
    const start = Math.max(0, end - (request.op === 'older' ? request.limit : 24))
    return { kind: 'page', instance, epoch: selectedEpoch, columns: 80, rows: 24, rowCount: saved.length, start, end, hasOlder: start > 0, records: saved.slice(start, end).map((text, i) => ({id:start+i+1,text,styles:[]})) }
  }
  return { kind: 'ack', instance, op: request.op }
}
const select = transport.instanceSelect!
transport.instanceSelect = async (...args) => {
  await new Promise(resolve => setTimeout(resolve, 1200))
  return select(...args)
}
const write = transport.write
transport.write = async (sid, input) => {
  if (input.every(byte => byte === 127 || byte === 8)) pending.get(sid)?.push(...new TextEncoder().encode('\b \b'.repeat(input.length)))
  else await write(sid, input)
}
function append(amount = 25) {
  const lines = Array.from({ length: amount }, () => `ROW-${++count} 桌面終端示範`)
  records.push(...lines)
  const text = lines.join('\r\n') + '\r\n'
  for (const queue of pending.values()) queue.push(...new TextEncoder().encode(text))
}
Object.assign(window, { __desktopPreview: { requests, append } })
</script>

<template><App :transport="transport" /></template>
