<script setup lang="ts">
import { inject, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import NativeTerminalReader from './NativeTerminalReader.vue'
import { NativeHistoryKey } from '../terminal/native'
import type { AppController, TabState } from '../state/controller'
import { TerminalFactoryKey, createDesktopXtermTerminal } from '../terminal/handle'

const props = defineProps<{
  tab: TabState
  active: boolean
  controller: AppController
  readerVisible?: boolean
}>()

const el = ref<HTMLElement | null>(null)
const nativeReader = inject(NativeHistoryKey, null) !== null
const factory = inject(TerminalFactoryKey, createDesktopXtermTerminal)
let observer: ResizeObserver | null = null
let disposed = false

function fitAndReport(): void {
  const term = props.tab.term
  if (!term || !props.active) return
  term.fit()
  props.controller.requestResize(props.tab, term.rows, term.cols)
}

onMounted(() => {
  if (!el.value) return
  // The instance stays mounted for the tab's whole life (v-show hides it);
  // buffers keep draining in the background.
  const handle = factory(el.value)
  handle.fit()
  props.controller.attachTerminal(props.tab, handle)
  if (props.active) handle.focus()

  observer = new ResizeObserver(() => fitAndReport())
  observer.observe(el.value)
})

watch(
  () => props.active,
  (now) => {
    if (!now || disposed) return
    void nextTick(() => {
      fitAndReport()
      props.tab.term?.focus()
    })
  }
)

onBeforeUnmount(() => {
  disposed = true
  observer?.disconnect()
  observer = null
  // The controller disposes the handle on tab close; guard anyway.
  if (props.tab.closed) props.tab.term?.dispose()
})
</script>

<template>
  <div class="pane" v-show="active">
    <div ref="el" class="term-host" />
    <NativeTerminalReader v-if="nativeReader && active && tab.status === 'live'"
      :key="tab.sessionId ?? tab.key" :terminal="tab.term"
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
