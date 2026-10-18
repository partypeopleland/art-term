<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { clampPan, fittedImage, zoomAt, type Point, type Size } from './preview-geometry'

const props = defineProps<{ src: string | null }>()
const emit = defineEmits<{ dimensions: [size: Size] }>()
const viewport = ref<HTMLElement | null>(null)
const image = ref<Size>({ width: 0, height: 0 })
const area = ref<Size>({ width: 0, height: 0 })
const zoom = ref(1)
const pan = ref<Point>({ x: 0, y: 0 })
const pointers = new Map<number, Point>()
let gesture: { distance: number; zoom: number; pan: Point; midpoint: Point } | null = null
let drag: Point | null = null
let observer: ResizeObserver | null = null
const fit = computed(() => fittedImage(image.value, area.value))
const imageStyle = computed(() => ({ width: `${fit.value.width}px`, height: `${fit.value.height}px`, transform: `translate(${pan.value.x}px, ${pan.value.y}px) scale(${zoom.value})` }))
function reset() { zoom.value = 1; pan.value = { x: 0, y: 0 }; pointers.clear(); drag = null; gesture = null }
watch(() => props.src, src => { if (!src) { image.value = { width: 0, height: 0 }; reset() } })
function loaded(event: Event) {
  const element = event.target as HTMLImageElement
  if (element.src !== props.src) return
  const size = { width: element.naturalWidth, height: element.naturalHeight }
  if (size.width !== image.value.width || size.height !== image.value.height) reset()
  image.value = size
  emit('dimensions', size)
}
function local(event: PointerEvent | WheelEvent): Point {
  const rect = viewport.value!.getBoundingClientRect()
  return { x: event.clientX - rect.left - rect.width / 2, y: event.clientY - rect.top - rect.height / 2 }
}
function wheel(event: WheelEvent) {
  event.preventDefault()
  event.stopPropagation()
  if (!image.value.width) return
  const next = Math.max(1, Math.min(4, zoom.value * Math.exp(-Math.sign(event.deltaY) * 0.12)))
  pan.value = zoomAt(pan.value, zoom.value, next, local(event), image.value, area.value)
  zoom.value = next
}
function midpoint(a: Point, b: Point): Point { return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } }
function distance(a: Point, b: Point): number { return Math.hypot(a.x - b.x, a.y - b.y) }
function pointerDown(event: PointerEvent) {
  if (event.pointerType === 'mouse' && event.button !== 0) return
  event.preventDefault()
  viewport.value?.setPointerCapture(event.pointerId)
  pointers.set(event.pointerId, local(event))
  const points = [...pointers.values()]
  if (points.length === 1) drag = points[0]
  if (points.length === 2) {
    gesture = { distance: distance(points[0], points[1]), zoom: zoom.value, pan: pan.value, midpoint: midpoint(points[0], points[1]) }
    drag = null
  }
}
function pointerMove(event: PointerEvent) {
  if (!pointers.has(event.pointerId)) return
  const current = local(event)
  const previous = pointers.get(event.pointerId)!
  pointers.set(event.pointerId, current)
  const points = [...pointers.values()]
  if (points.length === 2 && gesture) {
    const next = Math.max(1, Math.min(4, gesture.zoom * distance(points[0], points[1]) / Math.max(1, gesture.distance)))
    const middle = midpoint(points[0], points[1])
    const anchored = zoomAt(gesture.pan, gesture.zoom, next, gesture.midpoint, image.value, area.value)
    pan.value = clampPan({ x: anchored.x + middle.x - gesture.midpoint.x, y: anchored.y + middle.y - gesture.midpoint.y }, next, image.value, area.value)
    zoom.value = next
  } else if (drag && zoom.value > 1) {
    pan.value = clampPan({ x: pan.value.x + current.x - previous.x, y: pan.value.y + current.y - previous.y }, zoom.value, image.value, area.value)
    drag = current
  }
}
function pointerUp(event: PointerEvent) {
  pointers.delete(event.pointerId)
  gesture = null
  drag = pointers.size === 1 ? [...pointers.values()][0] : null
}
onMounted(() => {
  observer = new ResizeObserver(entries => {
    const rect = entries[0]?.contentRect
    if (!rect) return
    area.value = { width: rect.width, height: rect.height }
    pan.value = clampPan(pan.value, zoom.value, image.value, area.value)
  })
  if (viewport.value) observer.observe(viewport.value)
})
onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
  <div ref="viewport" class="zoom-viewport" data-testid="zoom-viewport" @wheel="wheel" @pointerdown="pointerDown" @pointermove="pointerMove" @pointerup="pointerUp" @pointercancel="pointerUp" @lostpointercapture="pointerUp">
    <img v-if="src" :src="src" :style="imageStyle" alt="放大的遠端桌面截圖" draggable="false" @load="loaded" />
    <span v-else class="zoom-empty">等待桌面截圖…</span>
  </div>
</template>

<style scoped>
.zoom-viewport { position: relative; width: 100%; height: 100%; overflow: hidden; display: flex; align-items: center; justify-content: center; background: #000; touch-action: none; user-select: none; cursor: grab; }
.zoom-viewport:active { cursor: grabbing; }
.zoom-viewport img { display: block; flex: none; max-width: none; max-height: none; object-fit: fill; pointer-events: none; transform-origin: center; }
.zoom-empty { color: #ddd; font-size: 13px; }
</style>
