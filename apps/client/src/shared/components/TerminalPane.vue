<script setup lang="ts">
import { inject, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import NativeTerminalReader from '../../mobile/NativeTerminalReader.vue'
import { NativeHistoryKey, HostTerminalViewport } from '../terminal/native'
import type { AppController, TabState } from '../state/controller'
import type { HostHistoryRequest } from '../terminal/native'
import { TerminalFactoryKey, createDesktopXtermTerminal, type TerminalHandle } from '../terminal/handle'

const props = withDefaults(defineProps<{
  tab: TabState
  /** Pane is on screen (its owning tab-view is visible). */
  active: boolean
  /** Pane owns keyboard focus; defaults to `active` for single-pane callers. */
  focused?: boolean
  controller: AppController
  readerVisible?: boolean
  nativeScroll?: boolean
  mirror?: boolean
  closable?: boolean
  closeLabel?: string
  onSelectionFinished?: (tab: TabState) => void
}>(), { focused: undefined })

const emit = defineEmits<{ close: []; activate: [] }>()

const isFocused = () => props.focused ?? props.active

const el = ref<HTMLElement | null>(null)
const nativeReader = inject(NativeHistoryKey, null) !== null
const factory = inject(TerminalFactoryKey, createDesktopXtermTerminal)
let observer: ResizeObserver | null = null
let disposed = false
let termHandle: TerminalHandle | null = null
const historyRequest = (request: HostHistoryRequest) => props.controller.historyRequest(props.tab, request)
const historyNotice = (text: string) => { if (!props.tab.closed) props.tab.notice = text }
let hostViewport: HostTerminalViewport | null = null
let viewportBarrier: Promise<void> = Promise.resolve()
let viewportTicket = 0
let selecting = false
let selectionTimer: ReturnType<typeof setTimeout> | undefined
function viewportEnabled(): boolean {
  return !!props.nativeScroll && !nativeReader && props.active && isFocused() && props.readerVisible !== false && props.tab.runtime
    && props.tab.status === 'live' && !document.hidden && !disposed
}
function syncViewport(): void {
  if (props.mirror) return
  const ticket = ++viewportTicket
  const previous = hostViewport
  hostViewport = null
  props.tab.term?.setHostViewport?.(null)
  const closing = previous?.close() ?? Promise.resolve()
  viewportBarrier = Promise.allSettled([viewportBarrier, closing]).then(() => undefined)
  if (!viewportEnabled() || !props.tab.term) return
  const session = props.tab.sessionId, terminal = props.tab.term
  void viewportBarrier.then(() => {
    if (ticket !== viewportTicket || !viewportEnabled() || props.tab.sessionId !== session) return
    hostViewport = new HostTerminalViewport(async request => {
      if (props.tab.sessionId !== session) return { kind: 'unsupported' }
      return props.controller.historyRequest(props.tab, request)
    }, historyNotice)
    terminal.setHostViewport?.(hostViewport)
    hostViewport.start()
  })
}
watch(() => [props.active, props.focused, props.readerVisible, props.nativeScroll, props.tab.term, props.tab.status, props.tab.sessionId], syncViewport)

function fitAndReport(): void {
  const term = termHandle
  if (!term || !props.active) return
  term.fit()
  if (!props.mirror) props.controller.requestResize(props.tab, term.rows, term.cols)
}

function focusOnPointer(event: PointerEvent): void {
  if (event.button !== 0) return
  emit('activate')
  // `focused` only updates next tick; focus directly so the clicked pane
  // owns keys immediately even when it was not the focused leaf before.
  if (!nativeReader && !props.mirror && props.active && props.tab.status === 'live') termHandle?.focus()
  if (props.onSelectionFinished && props.active) selecting = true
}

function finishSelection(event: PointerEvent): void {
  if (!selecting || event.button !== 0) return
  selecting = false
  clearTimeout(selectionTimer)
  // xterm completes its selection in its own pointerup handler first.
  selectionTimer = setTimeout(() => {
    if (!disposed && props.active) props.onSelectionFinished?.(props.tab)
  }, 0)
}

onMounted(() => {
  if (!el.value) return
  // The instance stays mounted for the tab's whole life (v-show hides it);
  // buffers keep draining in the background.
  const handle = factory(el.value)
  termHandle = handle
  handle.fit()
  if (props.mirror) props.controller.attachMirror(props.tab, handle)
  else props.controller.attachTerminal(props.tab, handle)
  if (props.active && isFocused() && !props.mirror) handle.focus()
  document.addEventListener('visibilitychange', syncViewport)
  window.addEventListener('pointerup', finishSelection)
  window.addEventListener('pointercancel', finishSelection)
  syncViewport()

  observer = new ResizeObserver(() => fitAndReport())
  observer.observe(el.value)
})

watch(
  () => [props.active, props.focused],
  ([now, focused]) => {
    if (!now || disposed) return
    void nextTick(() => {
      fitAndReport()
      if (focused ?? now) termHandle?.focus()
    })
  }
)

onBeforeUnmount(() => {
  disposed = true
  syncViewport()
  document.removeEventListener('visibilitychange', syncViewport)
  window.removeEventListener('pointerup', finishSelection)
  window.removeEventListener('pointercancel', finishSelection)
  clearTimeout(selectionTimer)
  observer?.disconnect()
  observer = null
  if (props.mirror) {
    if (termHandle) props.controller.detachMirror(props.tab, termHandle)
    termHandle = null
    return
  }
  // The controller disposes the handle on tab close; guard anyway.
  if (props.tab.closed) props.tab.term?.dispose()
})
</script>

<template>
  <div class="pane" :class="{ mirror: !!mirror }" :data-pane="mirror ? 'mirror' : 'primary'" v-show="active">
    <button v-if="closable" class="pane-close" type="button" :aria-label="closeLabel || '關閉窗格'" @click="emit('close')">×</button>
    <div v-if="mirror" class="mirror-badge">唯讀鏡像</div>
    <div ref="el" class="term-host" @pointerdown="focusOnPointer" />
    <NativeTerminalReader v-if="nativeReader && !mirror && active && tab.status === 'live'"
      :key="tab.sessionId ?? tab.key" :terminal="tab.term"
      :history-request="tab.runtime ? historyRequest : undefined" :history-notice="historyNotice"
      :visible="active && readerVisible !== false" />
    <div v-if="tab.status === 'connecting'" class="overlay">
      <span>連線中…</span>
    </div>
    <div v-else-if="tab.status === 'exited' || tab.status === 'error'" class="overlay">
      <div class="done">
        <div v-if="tab.status === 'error'" class="error">{{ tab.errorMsg }}</div>
        <div class="muted">
          本機連線已結束<span v-if="tab.exitCode !== null">（exit {{ tab.exitCode }}）</span>；
          遠端 Shell 或 tmux session 是否仍存在，以主機狀態為準。
        </div>
        <button class="btn" @click="controller.reopenTab(tab, false)">重新連線同一目標</button>
        <button class="btn" data-testid="refresh-targets" @click="void controller.refreshTargets()">重新整理並選擇目標</button>
      </div>
    </div>
  </div>
</template>
