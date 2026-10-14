<script setup lang="ts">
import { inject, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { NativeTerminalLine } from '../terminal/native-buffer'
import type { TerminalHandle } from '../terminal/handle'
import { NativeHistoryKey, type NativeHistoryEvent, type NativeHistorySnapshot } from '../terminal/native'

const props = defineProps<{ terminal: TerminalHandle | null; visible: boolean }>()
const factory = inject(NativeHistoryKey, null)
const native = factory?.()
const host = ref<HTMLElement | null>(null)
const failed = ref(false)
const readerId = `terminal-${crypto.randomUUID()}`
let disposed = false, opened = false, busy = false, dirty = false
let last = ''
let records: NativeTerminalLine[] = []
let timer: ReturnType<typeof setInterval> | undefined
let observer: ResizeObserver | undefined

function event(value: NativeHistoryEvent): void {
  if (disposed || value.readerId !== readerId) return
  // Native tap is deliberately the only touch event that can focus input.
  if (value.kind === 'tap' && props.visible && !document.hidden) props.terminal?.focus()
  if (value.kind === 'visibility') void sync()
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
      const buffer = visible ? props.terminal?.readStyledText?.() : undefined
      const rect = host.value?.getBoundingClientRect()
      const show = visible && buffer?.supported === true && !!rect && rect.width > 0 && rect.height > 0
      if (show && buffer) records = buffer.records
      const snapshot: NativeHistorySnapshot = {
        readerId, records, visible: show,
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
    props.terminal?.setNativeReaderActive?.(false)
    await native.close(readerId).catch(() => undefined)
    opened = false
  } finally { busy = false }
}
onMounted(() => {
  observer = new ResizeObserver(() => { void sync() })
  if (host.value) observer.observe(host.value)
  document.addEventListener('visibilitychange', sync)
  timer = setInterval(() => { if (props.visible && !document.hidden) void sync() }, 250)
  void sync()
})
watch(() => [props.visible, props.terminal], () => { void sync() })
onBeforeUnmount(() => {
  disposed = true
  if (timer) clearInterval(timer)
  observer?.disconnect()
  document.removeEventListener('visibilitychange', sync)
  props.terminal?.setNativeReaderActive?.(false)
  if (native) void native.close(readerId).catch(() => undefined)
})
</script>
<template>
  <div ref="host" class="native-terminal-host" aria-hidden="true" />
</template>
<style scoped>
.native-terminal-host { position: absolute; inset: 0; pointer-events: none; }
</style>
