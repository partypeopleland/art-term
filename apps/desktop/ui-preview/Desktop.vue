<script setup lang="ts">
import App from '../src/App.vue'
import { createPreviewTransport } from './transport'
import type { HostHistoryPage, HostHistoryRequest, HostHistoryReply } from '../src/terminal/native'

const transport = createPreviewTransport()
const requests: HostHistoryRequest[] = []
let epoch = 1, count = 1800
const counts = new Map([[epoch, count]])
const records = (total: number) => Array.from({ length: total }, (_, i) => ({ id: i + 1,
  text: `ROW-${i + 1} ` + (i % 7 === 0 ? '繁體中文與 English 長行自動換行 '.repeat(24) : '桌面終端示範'), styles: [] }))
const historyPage = (start: number, end: number, version = epoch): HostHistoryPage => ({ kind: 'page', instance: 'a'.repeat(32), epoch: version,
  columns: 80, rows: 24, rowCount: counts.get(version)!, start, end, hasOlder: start > 0, records: records(counts.get(version)!).slice(start, end) })
transport.runtimeHistory = async (_sid, request): Promise<HostHistoryReply> => {
  requests.push(request)
  if (request.op === 'viewport') return historyPage(Math.max(0, count - 24), count)
  if (request.op === 'older') {
    await new Promise(resolve => setTimeout(resolve, 150))
    return historyPage(Math.max(0, request.before - request.limit), request.before, request.epoch)
  }
  return { kind: 'ack', instance: 'a'.repeat(32), epoch, op: request.op }
}
const select = transport.instanceSelect!
transport.instanceSelect = async (...args) => {
  await new Promise(resolve => setTimeout(resolve, 1200))
  return select(...args)
}
const write = transport.write
transport.write = async (sid, input) => {
  await write(sid, input)
  append(1)
}
function append(amount = 25) { count += amount; epoch++; counts.set(epoch, count); for (const version of [...counts.keys()].slice(0, -2)) counts.delete(version) }
Object.assign(window, { __desktopPreview: { requests, append } })
</script>

<template><App :transport="transport" /></template>
