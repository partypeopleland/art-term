<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { HostHistoryWindow, type HostHistoryRequestFn, type HostHistoryPage } from '../terminal/native'
import type { NativeTerminalLine, TextStyle } from '../terminal/native-buffer'

const props = defineProps<{ request: HostHistoryRequestFn; visible: boolean; historyNotice?: (text: string) => void }>()
const emit = defineEmits<{ close: [] }>()
const viewport = ref<HTMLElement | null>(null)
const page = shallowRef<HostHistoryPage | null>(null)
const following = ref(true)
const busy = ref(false)
const notice = ref('')
const unavailable = ref(false)
const lineHeight = 21
let disposed = false
let timer: ReturnType<typeof setInterval> | undefined
const history = new HostHistoryWindow(props.request, text => { notice.value = text; props.historyNotice?.(text) })
const canReadOlder = computed(() => !!page.value && page.value.start > 0)
const canReadNewer = computed(() => !!page.value && page.value.end < page.value.rowCount)

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
function segments(line: NativeTerminalLine): { text: string; style?: Record<string, string | undefined> }[] {
  const result: { text: string; style?: Record<string, string | undefined> }[] = []
  let position = 0
  for (const run of line.styles) {
    if (run[0] > position) result.push({ text: line.text.slice(position, run[0]) })
    result.push({ text: line.text.slice(run[0], run[1]), style: style(run) })
    position = run[1]
  }
  if (position < line.text.length) result.push({ text: line.text.slice(position) })
  return result
}
async function display(): Promise<void> {
  if (disposed || !props.visible || document.hidden) return
  const host = viewport.value
  const anchorIndex = Math.floor((host?.scrollTop ?? 0) / lineHeight)
  const anchorId = page.value?.records[anchorIndex]?.id
  const offset = (host?.scrollTop ?? 0) % lineHeight
  page.value = history.visiblePage ? { ...history.visiblePage } : null
  const displayedEpoch = page.value ? HostHistoryWindow.key(page.value) : undefined
  following.value = !history.held
  unavailable.value = history.resolved && !history.visiblePage
  await nextTick()
  if (disposed || !props.visible || document.hidden) return
  if (host && page.value) {
    following.value = !history.held
    if (following.value) host.scrollTop = Math.max(0, host.scrollHeight - host.clientHeight)
    else if (anchorId !== undefined) {
      const index = page.value.records.findIndex(line => line.id === anchorId)
      host.scrollTop = index >= 0 ? index * lineHeight + offset : 0
    }
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
    if (!disposed && props.visible && !document.hidden) await history.displayed(displayedEpoch)
  }
}
async function refresh(): Promise<void> {
  if (disposed || busy.value || !props.visible || document.hidden) return
  busy.value = true
  try { await history.refresh(); await display() }
  finally { busy.value = false }
}
function hold(): void {
  if (!page.value || !props.visible || document.hidden) return
  history.hold(HostHistoryWindow.key(page.value))
  following.value = false
}
async function navigate(direction: 'older' | 'newer'): Promise<void> {
  if (disposed || busy.value || !page.value || !props.visible || document.hidden) return
  hold()
  busy.value = true
  const p = page.value
  try {
    await history.viewport({ readerId: 'windows-history', kind: 'viewport', sourceEpoch: HostHistoryWindow.key(p),
      anchorId: p.records[0]?.id, nearTop: direction === 'older', nearBottom: direction === 'newer', atBottom: false })
    await display()
  } finally { busy.value = false }
}
function scroll(): void {
  if (busy.value || !viewport.value || !page.value || !props.visible || document.hidden) return
  const host = viewport.value
  const atBottom = host.scrollTop + host.clientHeight >= host.scrollHeight - 2
  if (host.scrollTop <= lineHeight && canReadOlder.value) { void navigate('older'); return }
  if (atBottom && canReadNewer.value) { void navigate('newer'); return }
  if (!atBottom) hold()
  else if (page.value.end === page.value.rowCount) {
    history.held = false; following.value = true; void refresh()
  }
}
function wheel(event: WheelEvent): void {
  hold()
  if (event.deltaY < 0 && (viewport.value?.scrollTop ?? 0) <= lineHeight && canReadOlder.value) void navigate('older')
}
function latest(): void { history.held = false; following.value = true; void refresh() }
function visibility(): void { if (!document.hidden) void refresh() }
watch(() => props.visible, visible => { if (visible) void refresh() })
onMounted(() => {
  timer = setInterval(() => { void refresh() }, 500)
  document.addEventListener('visibilitychange', visibility)
  void refresh()
})
onBeforeUnmount(() => {
  disposed = true
  if (timer) clearInterval(timer)
  document.removeEventListener('visibilitychange', visibility)
  void history.close()
})
</script>

<template>
  <section v-show="visible" class="desktop-history" aria-label="終端歷史" data-testid="desktop-history">
    <header>
      <strong>終端歷史</strong>
      <button data-testid="history-older" :disabled="busy || !canReadOlder" @click="navigate('older')">較早紀錄</button>
      <button data-testid="history-newer" :disabled="busy || !canReadNewer" @click="navigate('newer')">較新紀錄</button>
      <button data-testid="history-latest" :disabled="busy || unavailable" @click="latest">最新內容</button>
      <span class="history-state">{{ following ? '跟隨最新' : '閱讀中' }}</span>
      <button class="history-close" data-testid="history-close" @click="emit('close')">返回終端</button>
    </header>
    <p v-if="notice" role="status">{{ notice }}</p>
    <p v-if="unavailable" role="status">此 Shell 尚不支援主機歷史，請返回終端；新建連線可使用新版歷史功能。</p>
    <p v-else-if="!page" role="status">載入歷史中…</p>
    <div ref="viewport" class="history-viewport" tabindex="0" aria-label="歷史內容，可選取複製" @scroll="scroll" @wheel.capture="wheel" @pointerdown="hold">
      <div v-for="line in page?.records ?? []" :key="line.id" class="history-line" :data-line-id="line.id"><span v-for="(part, index) in segments(line)" :key="index" :style="part.style">{{ part.text }}</span></div>
    </div>
  </section>
</template>

<style scoped>
.desktop-history { position: absolute; inset: 0; z-index: 40; display: flex; flex-direction: column; min-height: 0; background: var(--bg); color: var(--text); }
header { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
button { border: 1px solid var(--border); border-radius: 5px; padding: 5px 9px; background: var(--bg-raised); color: var(--text); font: inherit; cursor: pointer; }
button:disabled { opacity: .45; cursor: default; }
:is(button, .history-viewport):focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
.history-close { margin-left: auto; }
.history-state { color: var(--text-dim); font-size: 11px; }
p { margin: 8px 12px; color: var(--text-dim); }
.history-viewport { flex: 1; overflow: auto; min-height: 0; padding: 0 6px; user-select: text; font: 14px/21px Consolas, 'Cascadia Mono', 'Microsoft JhengHei', monospace; }
.history-line { height: 21px; min-width: max-content; white-space: pre; }
</style>
