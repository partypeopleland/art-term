<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { HostHistoryWindow, type HostHistoryRequestFn, type HostHistoryPage } from '../terminal/native'
import type { NativeTerminalLine, TextStyle } from '../terminal/native-buffer'
import type { TerminalHandle } from '../terminal/handle'
import { terminalTextLinks } from '../terminal/links'

const props = defineProps<{ request: HostHistoryRequestFn; visible: boolean; historyNotice?: (text: string) => void; terminal?: TerminalHandle | null }>()
const viewport = ref<HTMLElement | null>(null)
const page = shallowRef<HostHistoryPage | null>(null)
const busy = ref(false)
const notice = ref('')
const tailSpace = ref(0)
let tailSnapshot: { epoch: string; rows: number } | undefined
type Anchor = { id: string; offset: number }
let readingAnchor: Anchor | undefined
let observer: ResizeObserver | undefined
let restoring = false
let queuedDirection: 'older' | 'newer' | undefined
let disposed = false
let timer: ReturnType<typeof setInterval> | undefined
const history = new HostHistoryWindow(props.request, text => { notice.value = text; props.historyNotice?.(text) })

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
async function display(): Promise<void> {
  if (disposed || !props.visible || document.hidden) return
  const host = viewport.value
  const anchor = captureAnchor()
  page.value = history.visiblePage ? { ...history.visiblePage } : null
  const displayedEpoch = page.value ? HostHistoryWindow.key(page.value) : undefined
  if (page.value && displayedEpoch !== tailSnapshot?.epoch) tailSnapshot = { epoch: displayedEpoch!, rows: page.value.records.length }
  await nextTick()
  if (disposed || !props.visible || document.hidden) return
  if (host && page.value) {
    updateTailSpace()
    await nextTick()
    restoreAnchor(anchor)
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    if (!disposed && props.visible && !document.hidden) {
      await history.displayed(displayedEpoch)
      void history.prefetchOlder(() => !disposed && props.visible && !document.hidden)
    }
  }
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
  if (!history.held) host.scrollTop = Math.max(0, host.scrollHeight - host.clientHeight)
  else if (anchor) {
    const line = lines().find(line => line.dataset.lineId === anchor.id)
    host.scrollTop = line ? line.offsetTop + anchor.offset : 0
  }
  readingAnchor = captureAnchor()
  requestAnimationFrame(() => { restoring = false })
}
async function refresh(): Promise<void> {
  if (disposed || busy.value || !props.visible || document.hidden) return
  busy.value = true
  try { await history.refresh(); await display() }
  finally { busy.value = false; drainNavigation() }
}
function hold(): void {
  if (!page.value || !props.visible || document.hidden) return
  history.hold(HostHistoryWindow.key(page.value))
}
async function navigate(direction: 'older' | 'newer'): Promise<void> {
  if (disposed || !page.value || !props.visible || document.hidden) return
  if (busy.value) { queuedDirection = direction; return }
  hold()
  busy.value = true
  const p = page.value
  try {
    await history.viewport({ readerId: 'windows-history', kind: 'viewport', sourceEpoch: HostHistoryWindow.key(p),
      anchorId: p.records[0]?.id, nearTop: direction === 'older', nearBottom: direction === 'newer', atBottom: false })
    await display()
  } finally { busy.value = false; drainNavigation() }
}
function drainNavigation(): void {
  const direction = queuedDirection
  queuedDirection = undefined
  if (direction && !disposed && props.visible && !document.hidden) void navigate(direction)
}
function scroll(): void {
  if (restoring || busy.value || !viewport.value || !page.value || !props.visible || document.hidden) return
  const host = viewport.value
  readingAnchor = captureAnchor()
  const atBottom = host.scrollTop + host.clientHeight >= host.scrollHeight - 2
  if (!atBottom && host.scrollTop <= host.clientHeight && page.value.start > 0) { void navigate('older'); return }
  if (atBottom && page.value.end < page.value.rowCount) { void navigate('newer'); return }
  if (!atBottom) hold()
  else if (page.value.end === page.value.rowCount) {
    history.held = false; void refresh()
  }
}
function wheel(event: WheelEvent): void {
  if (event.ctrlKey) return
  if (event.deltaY < 0) {
    hold()
    const host = viewport.value
    if (host && host.scrollTop <= host.clientHeight && page.value && page.value.start > 0) void navigate('older')
  } else if (event.deltaY > 0) {
    const host = viewport.value
    if (host && host.scrollTop + host.clientHeight >= host.scrollHeight - 2) scroll()
  }
}
let press: { x: number; y: number } | undefined
function pointerDown(event: PointerEvent): void { press = { x: event.clientX, y: event.clientY }; hold() }
function click(event: MouseEvent): void {
  if (window.getSelection()?.toString() || !press || Math.hypot(event.clientX - press.x, event.clientY - press.y) > 5) return
  const url = (event.target as HTMLElement).closest<HTMLElement>('[data-url]')?.dataset.url
  if (url) void navigator.clipboard.writeText(url).then(() => { notice.value = '已複製連結' }, () => { notice.value = '複製失敗，請再點一次連結' })
  else props.terminal?.focus()
  const host = viewport.value
  if (host && page.value?.end === page.value?.rowCount && host.scrollTop + host.clientHeight >= host.scrollHeight - 2) {
    history.held = false; void refresh()
  }
  press = undefined
}
function visibility(): void { if (!document.hidden) void refresh() }
watch(() => props.visible, visible => { if (visible) void refresh() })
onMounted(() => {
  timer = setInterval(() => { void refresh() }, 500)
  document.addEventListener('visibilitychange', visibility)
  void refresh()
  observer = new ResizeObserver(async () => {
    if (disposed || busy.value || !props.visible || document.hidden || !viewport.value?.clientWidth) return
    updateTailSpace(); await nextTick()
    if (!disposed && props.visible) restoreAnchor(readingAnchor)
  })
  if (viewport.value) observer.observe(viewport.value)
})
onBeforeUnmount(() => {
  disposed = true
  if (timer) clearInterval(timer)
  observer?.disconnect()
  document.removeEventListener('visibilitychange', visibility)
  void history.close()
})
</script>

<template>
  <section v-show="visible && !!page" class="desktop-history" aria-label="終端內容" data-testid="desktop-history">
    <p v-if="notice" class="reader-notice" role="status">{{ notice }}</p>
    <div ref="viewport" class="history-viewport" aria-label="終端內容，可捲動與選取複製" @scroll="scroll" @wheel.capture="wheel" @pointerdown="pointerDown" @click="click">
      <div v-for="line in page?.records ?? []" :key="line.id" class="history-line" :data-line-id="line.id"><span v-for="(part, index) in segments(line)" :key="index" :style="part.style" :data-url="part.url">{{ part.text }}</span></div>
      <div aria-hidden="true" :style="{ height: `${tailSpace}px` }" />
    </div>
  </section>
</template>

<style scoped>
.desktop-history { position: absolute; inset: 0; display: flex; flex-direction: column; min-height: 0; background: var(--bg); color: var(--text); }
.reader-notice { position: absolute; right: 12px; top: 8px; margin: 0; padding: 6px; background: var(--bg-panel); color: var(--text-dim); pointer-events: none; }
.history-viewport { position: relative; flex: 1; overflow-x: hidden; overflow-y: auto; scrollbar-gutter: stable; overflow-anchor: none; min-width: 0; min-height: 0; padding: 4px 6px; user-select: text; font: 14px/17px Consolas, 'Cascadia Mono', 'Microsoft JhengHei', monospace; }
.history-line { min-height: 17px; white-space: pre-wrap; overflow-wrap: anywhere; }
[data-url] { cursor: pointer; }
[data-url]:hover { text-decoration: underline; }
</style>
