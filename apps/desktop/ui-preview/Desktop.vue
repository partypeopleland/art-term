<script setup lang="ts">
import App from '../src/App.vue'
import { createPreviewTransport } from './transport'
import type { HostHistoryRequest } from '../src/terminal/native'

const transport = createPreviewTransport()
const requests: HostHistoryRequest[] = []
const pending = new Map<string, number[]>()
let count = 0
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
  return { kind: 'ack', instance: 'a'.repeat(32), op: request.op }
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
  const text = Array.from({ length: amount }, () => `ROW-${++count} 桌面終端示範\r\n`).join('')
  for (const queue of pending.values()) queue.push(...new TextEncoder().encode(text))
}
Object.assign(window, { __desktopPreview: { requests, append } })
</script>

<template><App :transport="transport" /></template>
