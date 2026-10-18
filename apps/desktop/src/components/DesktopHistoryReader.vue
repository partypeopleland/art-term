<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { HostHistoryWindow, type HostHistoryRequestFn, type HostHistoryPage } from '../terminal/native'
import type { NativeTerminalLine, TextStyle } from '../terminal/native-buffer'
import type { TerminalHandle } from '../terminal/handle'
import { terminalTextLinks } from '../terminal/links'
const props = defineProps<{ request: HostHistoryRequestFn; visible: boolean; historyNotice?: (text: string) => void; terminal?: TerminalHandle | null }>()
const viewport = ref<HTMLElement | null>(null)
const page = shallowRef<HostHistoryPage | null>(null)
const notice = ref(''), tailSpace = ref(0)
let history: HostHistoryWindow | null = null
let tailSnapshot: { epoch: string; rows: number } | undefined
 type Anchor = { id: string; offset: number }
let readingAnchor: Anchor | undefined
let restoring = false, busy = false, disposed = false, generation = 0
let queuedDirection: 'older' | 'newer' | undefined
let retiring = Promise.resolve()
let initialWheelPixels = 0
let fallback = false
let observer: ResizeObserver | undefined
let timer: ReturnType<typeof setInterval> | undefined
const usable = () => !disposed && props.visible && !document.hidden
function close(focus = true): void {
  generation++; fallback = false; initialWheelPixels = 0; queuedDirection = undefined; busy = false
  const owned = history; history = null; page.value = null; tailSnapshot = undefined; readingAnchor = undefined
  if (owned) retiring = Promise.all([retiring, owned.close()]).then(() => {})
  if (focus && usable()) void nextTick(() => props.terminal?.focus())
}
function color(rgb: number): string | undefined {
  return rgb < 0 ? undefined : `#${rgb.toString(16).padStart(6, '0')}`
}
function style(run: TextStyle): Record<string, string | undefined> {
  const flags = run[4]
  const fg = flags & 32 ? color(run[3]) ?? 'var(--bg)' : color(run[2])
  const bg = flags & 32 ? color(run[2]) ?? 'var(--text)' : color(run[3])
  return { color: flags & 64 ? 'transparent' : fg, backgroundColor: bg,
    fontWeight: flags & 1 ? 'bold' : undefined, fontStyle: flags & 2 ? 'italic' : undefined,
    textDecoration: [flags & 4 ? 'underline' : '', flags & 8 ? 'line-through' : ''].filter(Boolean).join(' ') || undefined,
    opacity: flags & 16 ? '.5' : undefined }
}
function segments(line: NativeTerminalLine): { text: string; url?: string; style?: Record<string, string | undefined> }[] {
  const links = terminalTextLinks(line.text)
  const boundaries = [...new Set([0, line.text.length, ...line.styles.flatMap(run => [run[0], run[1]]), ...links.flatMap(link => [link.start, link.end])])].sort((a, b) => a - b)
  const result: { text: string; url?: string; style?: Record<string, string | undefined> }[] = []
  for (let i = 0; i < boundaries.length - 1; i++) {
    const start = boundaries[i], end = boundaries[i + 1]
    const run = line.styles.find(run => run[0] <= start && run[1] >= end)
    result.push({ text: line.text.slice(start, end), style: run ? style(run) : undefined, url: links.find(link => link.start <= start && link.end >= end)?.text })
  }
  return result
}
function lines(): HTMLElement[] { return Array.from(viewport.value?.querySelectorAll<HTMLElement>('.history-line') ?? []) }
function captureAnchor(): Anchor | undefined {
  const host = viewport.value
  if (!host) return
  const line = lines().find(line => line.offsetTop + line.offsetHeight > host.scrollTop + 4)
  return line ? { id: line.dataset.lineId!, offset: host.scrollTop - line.offsetTop } : undefined
}
function updateTailSpace(): void {
  const host = viewport.value, rows = lines().slice(-(tailSnapshot?.rows || 1))
  const first = rows[0], last = rows.at(-1)
  const contentHeight = first && last ? last.offsetTop + last.offsetHeight - first.offsetTop : 0
  tailSpace.value = host && page.value?.end === page.value?.rowCount
    ? Math.max(0, host.clientHeight - contentHeight - 8) : 0
}
function restoreAnchor(anchor: Anchor | undefined): void {
  const host = viewport.value
  if (!host) return
  restoring = true
  if (!history?.held) host.scrollTop = Math.max(0, host.scrollHeight - host.clientHeight)
  else if (anchor) {
    const line = lines().find(line => line.dataset.lineId === anchor.id)
    host.scrollTop = line ? line.offsetTop + anchor.offset : 0
  }
  readingAnchor = captureAnchor()
  requestAnimationFrame(() => { restoring = false })
}

async function display(owned: HostHistoryWindow, ticket: number): Promise<void> {
  if (!usable() || ticket !== generation || history !== owned) return
  const anchor = captureAnchor()
  page.value = owned.visiblePage ? { ...owned.visiblePage } : null
  if (!page.value) return
  const epoch = HostHistoryWindow.key(page.value)
  if (epoch !== tailSnapshot?.epoch) tailSnapshot = { epoch, rows: page.value.records.length }
  await nextTick()
  if (!usable() || ticket !== generation) return
  updateTailSpace(); await nextTick(); restoreAnchor(anchor)
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  if (usable() && ticket === generation) await owned.displayed(epoch)
}
async function navigate(direction: 'older' | 'newer'): Promise<void> {
  if (!usable()) return
  if (busy) { queuedDirection = direction; return }
  busy = true
  const ticket = generation
  const owned = history ?? (history = new HostHistoryWindow(props.request, text => { if (ticket === generation) { notice.value = text; props.historyNotice?.(text) } }))
  try {
    await retiring
    if (!usable() || ticket !== generation) return
    const initial = !owned.page
    if (initial) {
      await owned.refresh()
      if (!usable() || ticket !== generation) return
      await display(owned, ticket)
    }
    if (!owned.visiblePage) {
      const unavailable = owned.unavailable
      close(false)
      fallback = unavailable
      if (unavailable) props.historyNotice?.('此 Shell 暫不支援主機歷史；即時終端仍可操作。')
      return
    }
    if (!usable() || ticket !== generation) return
    const p = owned.visiblePage
    owned.hold(HostHistoryWindow.key(p))
    if (direction === 'newer' && p.end === p.rowCount) { close(); return }
    await owned.viewport({ readerId: 'windows-history', kind: 'viewport', sourceEpoch: HostHistoryWindow.key(p), anchorId: p.records[0]?.id, nearTop: direction === 'older', nearBottom: direction === 'newer', atBottom: false })
    await display(owned, ticket)
    if (initial && usable() && ticket === generation && viewport.value) { viewport.value.scrollTop = Math.max(0, viewport.value.scrollTop + initialWheelPixels); initialWheelPixels = 0; readingAnchor = captureAnchor() }
  } finally {
    if (ticket === generation) {
      busy = false
      const direction = queuedDirection; queuedDirection = undefined
      if (direction && usable()) void navigate(direction)
    }
  }
}
function wheel(event: WheelEvent): void {
  if (event.ctrlKey || fallback || !usable() || !event.deltaY) return
  if (!page.value) {
    if (event.deltaY < 0) { event.preventDefault(); event.stopPropagation(); initialWheelPixels = Math.max(-1000, initialWheelPixels + event.deltaY * (event.deltaMode === 1 ? 17 : event.deltaMode === 2 ? 400 : 1)); void navigate('older') }
    return
  }
  const host = viewport.value
  history?.hold(HostHistoryWindow.key(page.value))
  if (event.deltaY < 0 && host && host.scrollTop <= host.clientHeight && page.value.start > 0) void navigate('older')
  else if (event.deltaY > 0 && host && host.scrollTop + host.clientHeight >= host.scrollHeight - 2) void navigate('newer')
}
function scroll(): void {
  if (restoring || busy || !usable() || !viewport.value || !page.value) return
  const host = viewport.value
  readingAnchor = captureAnchor(); history?.hold(HostHistoryWindow.key(page.value))
  if (host.scrollTop <= host.clientHeight && page.value.start > 0) void navigate('older')
  else if (host.scrollTop + host.clientHeight >= host.scrollHeight - 2) void navigate('newer')
}
let press: { x: number; y: number } | undefined
function pointerDown(event: PointerEvent): void { press = { x: event.clientX, y: event.clientY } }
function click(event: MouseEvent): void {
  const start = press; press = undefined
  if (window.getSelection()?.toString() || !start || Math.hypot(event.clientX - start.x, event.clientY - start.y) > 5) return
  const url = (event.target as HTMLElement).closest<HTMLElement>('[data-url]')?.dataset.url
  if (url) void navigator.clipboard.writeText(url).then(() => { notice.value = '已複製連結' }, () => { notice.value = '複製失敗，請再點一次連結' })
}
function visibility(): void { if (document.hidden) close(false) }
watch(() => props.visible, visible => { if (!visible) close(false) })
watch(() => props.request, () => close(false))
defineExpose({ wheel, close })
onMounted(() => {
  document.addEventListener('visibilitychange', visibility)
  // A held snapshot needs only its existing keep contract, never live polling.
  timer = setInterval(() => { if (usable() && history?.held) void history.refresh() }, 30000)
  observer = new ResizeObserver(async () => { if (!usable() || !page.value || busy) return; updateTailSpace(); await nextTick(); restoreAnchor(readingAnchor) })
  if (viewport.value) observer.observe(viewport.value)
})
onBeforeUnmount(() => { disposed = true; close(false); observer?.disconnect(); if (timer) clearInterval(timer); document.removeEventListener('visibilitychange', visibility) })
</script>
<template>
  <section v-show="visible && !!page" class="desktop-history" aria-label="終端內容" data-testid="desktop-history">
    <button class="history-live" type="button" data-testid="history-live" @click="close()">返回即時</button>
    <p v-if="notice" class="reader-notice" role="status">{{ notice }}</p>
    <div ref="viewport" class="history-viewport" aria-label="終端內容，可捲動與選取複製" @scroll="scroll" @wheel.capture="wheel" @pointerdown="pointerDown" @click="click">
      <div v-for="line in page?.records ?? []" :key="line.id" class="history-line" :data-line-id="line.id"><span v-for="(part, index) in segments(line)" :key="index" :style="part.style" :data-url="part.url">{{ part.text }}</span></div>
      <div aria-hidden="true" :style="{ height: `${tailSpace}px` }" />
    </div>
  </section>
</template>

<style scoped>
.desktop-history { position: absolute; inset: 0; display: flex; flex-direction: column; min-height: 0; background: var(--bg); color: var(--text); }
.history-live { position: absolute; right: 12px; bottom: 8px; z-index: 1; background: var(--bg-panel); color: var(--text); border: 1px solid var(--border); padding: 6px; cursor: pointer; }
.reader-notice { position: absolute; right: 12px; top: 8px; margin: 0; padding: 6px; background: var(--bg-panel); color: var(--text-dim); pointer-events: none; }
.history-viewport { position: relative; flex: 1; overflow-x: hidden; overflow-y: auto; scrollbar-gutter: stable; overflow-anchor: none; min-width: 0; min-height: 0; padding: 4px 6px; user-select: text; font: 14px/17px Consolas, 'Cascadia Mono', 'Microsoft JhengHei', monospace; }
.history-line { min-height: 17px; white-space: pre-wrap; overflow-wrap: anywhere; }
[data-url] { cursor: pointer; }
[data-url]:hover { text-decoration: underline; }
</style>
