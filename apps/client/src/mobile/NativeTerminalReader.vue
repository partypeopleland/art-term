<script setup lang="ts">
import { inject, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { NATIVE_PAGE_LINES, NATIVE_WINDOW_LINES, type NativeTerminalLine } from '../shared/terminal/native-buffer'
import type { TerminalHandle } from '../shared/terminal/handle'
import { HostHistoryWindow, NativeHistoryKey, NativeSwipeKey, type HostHistoryRequestFn, type NativeHistoryEvent, type NativeHistorySnapshot } from '../shared/terminal/native'

const props = defineProps<{ terminal: TerminalHandle | null; visible: boolean; historyRequest?: HostHistoryRequestFn; historyNotice?: (text: string) => void }>()
const factory = inject(NativeHistoryKey, null)
const native = factory?.()
const swipe = inject(NativeSwipeKey, null)
const host = ref<HTMLElement | null>(null)
const failed = ref(false)
const remoteLoading = ref(false)
const readerId = `terminal-${crypto.randomUUID()}`
let disposed = false, opened = false, busy = false, dirty = false
let last = ''
let records: NativeTerminalLine[] = []
let firstId: number | undefined
let limit = NATIVE_PAGE_LINES
let followLatest = true, hasOlder = false, hasNewer = false, requestedPage = false
let timer: ReturnType<typeof setInterval> | undefined
let observer: ResizeObserver | undefined
let remote: HostHistoryWindow | undefined

function event(value: NativeHistoryEvent): void {
  if (disposed || value.readerId !== readerId) return
  if (value.kind === 'swipe') {
    if (props.visible && !document.hidden && value.phase && value.dx !== undefined) swipe?.({ phase: value.phase, dx: value.dx })
    return
  }
  // Native tap is deliberately the only touch event that can focus input.
  if (value.kind === 'tap' && props.visible && !document.hidden) props.terminal?.focus()
  if (value.kind === 'visibility') void sync()
  if (remote?.visiblePage && props.visible && !document.hidden) {
    if (value.kind === 'reading') { remote.hold(value.sourceEpoch); void sync(); return }
    if (value.kind === 'position') { void remote.displayed(value.sourceEpoch); return }
    if (value.kind === 'viewport') { void remote.viewport(value).then(() => sync()); return }
  }
  if (value.kind !== 'viewport' || !props.visible || document.hidden || !records.length) return
  if (value.anchorId !== undefined && !records.some(r => r.id === value.anchorId)) return
  if (value.nearTop && hasOlder) {
    firstId = records[0].id - NATIVE_PAGE_LINES
    limit = Math.min(limit + NATIVE_PAGE_LINES, NATIVE_WINDOW_LINES)
    followLatest = false; requestedPage = true
  } else if (value.nearBottom && hasNewer) {
    firstId = records[0].id + Math.max(0, limit + NATIVE_PAGE_LINES - NATIVE_WINDOW_LINES)
    limit = Math.min(limit + NATIVE_PAGE_LINES, NATIVE_WINDOW_LINES)
    followLatest = false; requestedPage = true
  } else if (value.atBottom && !hasNewer) {
    firstId = undefined; limit = NATIVE_PAGE_LINES; followLatest = true
  } else {
    firstId = records[0].id; followLatest = false
  }
  void sync()
}
async function sync(): Promise<void> {
  if (!native || disposed || failed.value) return
  dirty = true
  if (busy) return
  busy = true
  try {
    while (dirty && !disposed) {
      dirty = false
      const visible = props.visible && !document.hidden
      remoteLoading.value = visible && !!remote && !remote.resolved
      if (visible) await remote?.refresh()
      remoteLoading.value = visible && !!remote && !remote.resolved
      if (disposed) break
      const page = remote?.visiblePage
      const buffer = visible && !remoteLoading.value ? page ? { records: page.records, supported: true, hasOlder: page.start > 0, hasNewer: page.end < page.rowCount }
        : props.terminal?.readStyledText?.({ firstId: followLatest ? undefined : firstId, limit }) : undefined
      const rect = host.value?.getBoundingClientRect()
      const show = visible && buffer?.supported === true && !!rect && rect.width > 0 && rect.height > 0
      if (show && buffer) {
        // Retain the bounded page being read if xterm has since evicted it.
        // Only explicit navigation may replace it with a newer available page.
        if (!page && !followLatest && !requestedPage && records.length && buffer.records.length
          && buffer.records[0].id > records[0].id) {
          hasOlder = false; hasNewer = true
        } else {
          records = buffer.records
          hasOlder = buffer.hasOlder === true; hasNewer = buffer.hasNewer === true
        }
        requestedPage = false
      }
      const snapshot: NativeHistorySnapshot = {
        foreground: parseInt((host.value ? getComputedStyle(host.value).getPropertyValue('--text').trim() : '').replace('#', '') || 'd7d7dc', 16),
        background: parseInt((host.value ? getComputedStyle(host.value).getPropertyValue('--bg').trim() : '').replace('#', '') || '16171b', 16),
        readerId, records, visible: show, followLatest: page ? !remote!.held : followLatest,
        ...(page ? { sourceEpoch: remote!.sourceEpoch } : {}),
        bounds: { x: Math.max(0, rect?.x ?? 0), y: Math.max(0, rect?.y ?? 0), width: rect?.width ?? 0, height: rect?.height ?? 0, viewportWidth: window.innerWidth }
      }
      if (!opened && !show) { props.terminal?.setNativeReaderActive?.(false); continue }
      if (!opened) {
        await native.open(snapshot, event)
        opened = true
        if (disposed) { await native.close(readerId); break }
      }
      const key = JSON.stringify(snapshot)
      if (key !== last) { await native.update(snapshot); last = key }
      props.terminal?.setNativeReaderActive?.(show)
    }
  } catch {
    failed.value = true
    void remote?.close()
    props.terminal?.setNativeReaderActive?.(false)
    await native.close(readerId).catch(() => undefined)
    opened = false
  } finally { busy = false }
}
onMounted(() => {
  if (props.historyRequest) remote = new HostHistoryWindow(props.historyRequest, props.historyNotice)
  observer = new ResizeObserver(() => { void sync() })
  if (host.value) observer.observe(host.value)
  document.addEventListener('visibilitychange', sync)
  timer = setInterval(() => { if (props.visible && !document.hidden) void sync() }, 250)
  void sync()
})
watch(() => props.visible, () => { void sync() })
watch(() => props.terminal, () => {
  firstId = undefined; limit = NATIVE_PAGE_LINES; followLatest = true
  hasOlder = false; hasNewer = false; requestedPage = false; records = []; last = ''
  void sync()
})
onBeforeUnmount(() => {
  disposed = true
  void remote?.close()
  if (timer) clearInterval(timer)
  observer?.disconnect()
  document.removeEventListener('visibilitychange', sync)
  props.terminal?.setNativeReaderActive?.(false)
  if (native) void native.close(readerId).catch(() => undefined)
})
</script>
<template>
  <div ref="host" class="native-terminal-host" aria-hidden="true">
    <div v-if="remoteLoading" class="history-loading" role="status">載入畫面中…</div>
  </div>
</template>
<style scoped>
.native-terminal-host { position: absolute; inset: 0; pointer-events: none; }
.history-loading { pointer-events: auto; position: absolute; inset: 0; display: grid; place-items: center; background: var(--bg); color: var(--text); }
</style>
