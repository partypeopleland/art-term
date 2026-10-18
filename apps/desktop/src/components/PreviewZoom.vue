<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { TabState } from '../state/controller'
import { panelImageSize, type Point, type Size } from './preview-geometry'
import ZoomViewport from './ZoomViewport.vue'

const props = defineProps<{ tab: TabState; mode: 'windows' | 'android'; screenKey: string }>()
const emit = defineEmits<{ close: [] }>()
const root = ref<HTMLElement | null>(null)
const available = ref<Size>({ width: 0, height: 0 })
const source = ref<Size>({ width: 0, height: 0 })
const position = ref<Point>({ x: 0, y: 0 })
const imageSize = computed(() => panelImageSize(source.value, { width: Math.max(0, available.value.width - 18), height: Math.max(0, available.value.height - 58) }))
const panelWidth = computed(() => (imageSize.value.width || 280) + 2)
const panelHeight = computed(() => (imageSize.value.height || 140) + 42)
const panelStyle = computed(() => ({ width: `${panelWidth.value}px`, height: `${panelHeight.value}px`, left: `${position.value.x}px`, top: `${position.value.y}px` }))
let observer: ResizeObserver | null = null
let drag: { x: number; y: number; left: number; top: number } | null = null
function clampPosition() {
  position.value = {
    x: Math.max(0, Math.min(position.value.x, available.value.width - panelWidth.value)),
    y: Math.max(0, Math.min(position.value.y, available.value.height - panelHeight.value))
  }
}
function measure() {
  const rect = root.value?.getBoundingClientRect()
  if (!rect) return
  available.value = { width: rect.width, height: rect.height }
  clampPosition()
}
function dimensions(size: Size) {
  const first = !source.value.width
  source.value = size
  void nextTick(() => {
    if (first) position.value = { x: (available.value.width - panelWidth.value) / 2, y: (available.value.height - panelHeight.value) / 2 }
    clampPosition()
  })
}
watch(() => props.screenKey, () => { source.value = { width: 0, height: 0 }; position.value = { x: 0, y: 0 } })
watch([panelWidth, panelHeight], () => { void nextTick(clampPosition) })
function titleDown(event: PointerEvent) {
  if (event.button !== 0) return
  event.preventDefault()
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  drag = { x: event.clientX, y: event.clientY, left: position.value.x, top: position.value.y }
}
function titleMove(event: PointerEvent) {
  if (!drag) return
  position.value = { x: drag.left + event.clientX - drag.x, y: drag.top + event.clientY - drag.y }
  clampPosition()
}
onMounted(() => {
  if (props.mode === 'windows' && root.value) {
    observer = new ResizeObserver(measure)
    observer.observe(root.value)
    measure()
  }
})
onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
  <div v-if="mode === 'windows'" ref="root" class="zoom-layer windows-zoom" data-testid="windows-preview-zoom">
    <section class="zoom-panel" :style="panelStyle" aria-label="桌面放大檢視">
      <header class="zoom-title" @pointerdown="titleDown" @pointermove="titleMove" @pointerup="drag = null" @pointercancel="drag = null" @lostpointercapture="drag = null">
        <span>{{ tab.instanceDesktop?.name || '桌面預覽' }}</span>
        <button type="button" class="zoom-close" aria-label="關閉放大檢視" @pointerdown.stop @click="emit('close')">×</button>
      </header>
      <div class="zoom-image"><ZoomViewport :key="screenKey" :src="tab.shot?.src || null" @dimensions="dimensions" /></div>
      <div v-if="tab.shotError || tab.desktopError || tab.status !== 'live'" class="zoom-error" role="status">更新失敗</div>
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
</style>
