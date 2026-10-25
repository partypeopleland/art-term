<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { TabState } from '../state/controller'
import type { Point, Size } from './preview-geometry'
import { boundPanel, defaultPanelSize, resizeEdges, resizePanel, type PanelRect, type ResizeEdge } from './preview-window'
import ZoomViewport from './ZoomViewport.vue'

const props = defineProps<{ tab: TabState; mode: 'windows' | 'android'; screenKey: string }>()
const emit = defineEmits<{ close: [] }>()
const root = ref<HTMLElement | null>(null)
const available = ref<Size>({ width: 0, height: 0 })
const minimum = ref<Size>({ width: 282, height: 182 })
const panel = ref<{ kind: 'floating' | 'maximized'; rect: PanelRect }>({ kind: 'floating', rect: { x: 0, y: 0, width: 282, height: 182 } })
const visibleRect = computed<PanelRect>(() => panel.value.kind === 'maximized'
  ? { x: 0, y: 0, ...available.value }
  : boundPanel(panel.value.rect, minimum.value, available.value))
const panelStyle = computed(() => {
  const { x, y, width, height } = visibleRect.value
  return { width: `${width}px`, height: `${height}px`, left: `${x}px`, top: `${y}px` }
})
type Gesture = { kind: 'idle' }
  | { kind: 'click' | 'move'; pointerId: number; start: Point; rect: PanelRect; moved: boolean }
  | { kind: 'resize'; pointerId: number; start: Point; rect: PanelRect; edge: ResizeEdge }
let gesture: Gesture = { kind: 'idle' }
const activePointers = new Set<number>()
let observer: ResizeObserver | null = null
let initialized = false
function measure() {
  const rect = root.value?.getBoundingClientRect()
  if (rect) available.value = { width: rect.width, height: rect.height }
}
function dimensions(size: Size) {
  if (initialized || !size.width || !size.height) return
  minimum.value = defaultPanelSize(size, available.value)
  const { width, height } = minimum.value
  panel.value = { kind: 'floating', rect: { width, height, x: (available.value.width - width) / 2, y: (available.value.height - height) / 2 } }
  initialized = true
}
watch(() => props.screenKey, () => {
  initialized = false
  activePointers.clear()
  minimum.value = defaultPanelSize({ width: 0, height: 0 }, available.value)
  panel.value = { kind: 'floating', rect: { ...minimum.value, x: 0, y: 0 } }
  gesture = { kind: 'idle' }
})
function panelDown(event: PointerEvent) {
  activePointers.add(event.pointerId)
  if (activePointers.size !== 1 || gesture.kind !== 'idle' || event.pointerType !== 'mouse' || !event.isPrimary || event.button !== 0) {
    gesture = { kind: 'idle' }
    return
  }
  if (event.target instanceof Element && event.target.closest('button, .zoom-resize')) return
  gesture = { kind: 'click', pointerId: event.pointerId, start: { x: event.clientX, y: event.clientY }, rect: visibleRect.value, moved: false }
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.setPointerCapture(event.pointerId)
}
function titleDown(event: PointerEvent) {
  if (gesture.kind !== 'click' || panel.value.kind !== 'floating') return
  event.preventDefault()
  gesture = { ...gesture, kind: 'move' }
}
function resizeDown(event: PointerEvent, edge: ResizeEdge) {
  if (event.button !== 0 || !event.isPrimary || panel.value.kind !== 'floating') return
  event.preventDefault()
  gesture = { kind: 'resize', pointerId: event.pointerId, start: { x: event.clientX, y: event.clientY }, rect: visibleRect.value, edge }
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.setPointerCapture(event.pointerId)
}
function panelMove(event: PointerEvent) {
  if (gesture.kind === 'idle' || gesture.pointerId !== event.pointerId) return
  const delta = { x: event.clientX - gesture.start.x, y: event.clientY - gesture.start.y }
  if (gesture.kind === 'resize') {
    panel.value = { kind: 'floating', rect: resizePanel(gesture.rect, gesture.edge, delta, minimum.value, available.value) }
  } else {
    gesture.moved ||= delta.x !== 0 || delta.y !== 0
    if (gesture.kind === 'move') panel.value = { kind: 'floating', rect: boundPanel({ ...gesture.rect, x: gesture.rect.x + delta.x, y: gesture.rect.y + delta.y }, minimum.value, available.value) }
  }
}
function panelUp(event: PointerEvent) {
  activePointers.delete(event.pointerId)
  if (gesture.kind === 'idle' || gesture.pointerId !== event.pointerId) return
  const current = gesture
  gesture = { kind: 'idle' }
  if (current.kind === 'resize' || current.moved || event.button !== 0 || event.clientX !== current.start.x || event.clientY !== current.start.y) return
  if (event.target instanceof Element && event.target.closest('button, .zoom-resize')) return
  panel.value = { kind: panel.value.kind === 'floating' ? 'maximized' : 'floating', rect: panel.value.rect }
}
function cancelGesture() { gesture = { kind: 'idle' } }
function endPointer(event: PointerEvent) { activePointers.delete(event.pointerId) }
function cancelPointer(event: PointerEvent) { endPointer(event); cancelGesture() }
onMounted(() => {
  window.addEventListener('pointerup', endPointer, true)
  window.addEventListener('pointercancel', cancelPointer, true)
  if (props.mode === 'windows' && root.value) {
    observer = new ResizeObserver(measure)
    observer.observe(root.value)
    measure()
  }
})
onBeforeUnmount(() => {
  observer?.disconnect()
  window.removeEventListener('pointerup', endPointer, true)
  window.removeEventListener('pointercancel', cancelPointer, true)
})
</script>

<template>
  <div v-if="mode === 'windows'" ref="root" class="zoom-layer windows-zoom" data-testid="windows-preview-zoom">
    <section class="zoom-panel" :class="{ maximized: panel.kind === 'maximized' }" :style="panelStyle" @pointerdown.capture="panelDown" @pointermove.capture="panelMove" @pointerup.capture="panelUp" @pointercancel.capture="cancelPointer" @lostpointercapture.capture="cancelGesture" @wheel.capture="cancelGesture" aria-label="桌面放大檢視">
      <header class="zoom-title" @pointerdown="titleDown">
        <span>{{ tab.instanceDesktop?.name || '桌面預覽' }}</span>
        <button type="button" class="zoom-close" aria-label="關閉放大檢視" @pointerdown.stop @click="emit('close')">×</button>
      </header>
      <div class="zoom-image"><ZoomViewport :key="screenKey" :src="tab.shot?.src || null" @dimensions="dimensions" /></div>
      <div v-if="tab.shotError || tab.desktopError || tab.status !== 'live'" class="zoom-error" role="status">更新失敗</div>
      <template v-if="panel.kind === 'floating'"><div v-for="edge in resizeEdges" :key="edge" class="zoom-resize" :class="`resize-${edge}`" :data-resize="edge" @pointerdown="resizeDown($event, edge)" /></template>
    </section>
  </div>
  <div v-else class="zoom-layer mobile-zoom" data-testid="mobile-preview-zoom">
    <ZoomViewport :key="screenKey" :src="tab.shot?.src || null" />
    <div v-if="tab.shotError || tab.desktopError || tab.status !== 'live'" class="zoom-error" role="status">更新失敗</div>
    <button type="button" class="zoom-close mobile-close" aria-label="關閉放大檢視" @click="emit('close')">×</button>
  </div>
</template>

<style scoped>
.zoom-layer { position: absolute; inset: 0; z-index: 20; pointer-events: none; }
.zoom-panel { position: absolute; display: flex; flex-direction: column; box-sizing: border-box; min-width: 0; overflow: hidden; border: 1px solid var(--border); border-radius: 9px; background: var(--bg-panel); box-shadow: 0 14px 38px #0009; pointer-events: auto; }
.zoom-title { height: 40px; flex: none; display: flex; align-items: center; gap: 8px; padding: 0 4px 0 12px; cursor: move; touch-action: none; user-select: none; }
.zoom-title span { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.zoom-close { display: grid; place-items: center; flex: none; width: 36px; height: 36px; border: 0; border-radius: 6px; background: transparent; color: var(--text); font-size: 25px; cursor: pointer; }
.zoom-close:hover { background: var(--bg-hover); }
.zoom-image { flex: 1; min-height: 0; }
.zoom-error { position: absolute; bottom: 8px; left: 8px; padding: 4px 8px; border-radius: 5px; background: #732626e8; color: white; font-size: 12px; pointer-events: none; }
.mobile-zoom { background: #000; pointer-events: auto; }
.mobile-close { position: absolute; top: max(12px, env(safe-area-inset-top)); right: max(12px, env(safe-area-inset-right)); width: 48px; height: 48px; background: #333c; color: #fff; z-index: 1; }
.maximized { border-radius: 0; }
.maximized .zoom-title { cursor: default; }
.zoom-resize { position: absolute; touch-action: none; z-index: 2; }
.resize-n, .resize-s { left: 8px; right: 8px; height: 6px; cursor: ns-resize; }
.resize-n { top: 0; } .resize-s { bottom: 0; }
.resize-e, .resize-w { top: 8px; bottom: 8px; width: 6px; cursor: ew-resize; }
.resize-e { right: 0; } .resize-w { left: 0; }
.resize-ne, .resize-nw, .resize-se, .resize-sw { width: 10px; height: 10px; }
.resize-ne { top: 0; right: 0; cursor: nesw-resize; }
.resize-nw { top: 0; left: 0; cursor: nwse-resize; }
.resize-se { bottom: 0; right: 0; cursor: nwse-resize; }
.resize-sw { bottom: 0; left: 0; cursor: nesw-resize; }
</style>
