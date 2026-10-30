import { computed, nextTick, onBeforeUnmount, ref, type Ref } from 'vue'
import type { NativeSwipeEvent } from '../shared/terminal/native'

export type Workspace = 'home' | 'list' | 'session'
const pages: Workspace[] = ['home', 'list', 'session']

/** One gesture owner for DOM and Android-native terminal drags. */
export function useWorkspacePager(root: Ref<HTMLElement | null>, page: Ref<Workspace>, activate: (page: Workspace) => void, canStart: () => boolean) {
  const offset = ref(0)
  const dragging = ref(false)
  const animating = ref(false)
  const nativeDragging = ref(false)
  const origin = ref<Workspace>('list')
  const width = ref(390)
  let pointer: number | null = null
  let startX = 0, startY = 0, startedAt = 0, clickUntil = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  const visible = computed(() => dragging.value || animating.value)
  const progress = computed(() => visible.value
    ? Math.max(0, Math.min(pages.length - 1, pages.indexOf(origin.value) - offset.value / width.value))
    : pages.indexOf(page.value))
  const indicatorStyle = computed(() => ({ transform: `translateX(${progress.value * 100}%)` }))
  function transform(target: Workspace): string {
    return visible.value ? `translateX(${(pages.indexOf(target) - pages.indexOf(origin.value)) * width.value + offset.value}px)` : 'translateX(0)'
  }
  function start(): void {
    origin.value = page.value
    width.value = root.value?.clientWidth || 390
    startedAt = performance.now()
  }
  function cancel(): void {
    if (timer) clearTimeout(timer)
    timer = undefined
    pointer = null
    offset.value = 0
    dragging.value = animating.value = nativeDragging.value = false
  }
  function settle(commit: boolean, destination: Workspace): void {
    animating.value = true
    dragging.value = nativeDragging.value = false
    offset.value = commit ? (pages.indexOf(origin.value) - pages.indexOf(destination)) * width.value : 0
    timer = setTimeout(() => {
      if (commit) activate(destination)
      animating.value = false
      offset.value = 0
      timer = undefined
    }, 200)
  }
  function finish(cancelled: boolean): void {
    if (!dragging.value) return
    clickUntil = performance.now() + 400
    const distance = Math.abs(offset.value)
    const commit = !cancelled && (distance > width.value * .25 || (distance > 45 && performance.now() - startedAt < 300))
    const destination = pages[pages.indexOf(origin.value) + (offset.value < 0 ? 1 : -1)]
    settle(commit && !!destination, destination ?? origin.value)
  }
  function begin(event: PointerEvent): void {
    if (event.isPrimary === false || event.button !== 0 || animating.value || !canStart()) return
    const target = event.target instanceof Element ? event.target : null
    if (target?.closest('input, textarea, select, [role="dialog"], .keybar, .tabbar, .mobile-scroll-track, .drag-handle')) return
    pointer = event.pointerId
    startX = event.clientX; startY = event.clientY
    start()
  }
  function move(event: PointerEvent): void {
    if (event.pointerId !== pointer) return
    const dx = event.clientX - startX, dy = event.clientY - startY
    if (!dragging.value) {
      if (Math.abs(dy) > 8 && Math.abs(dy) > Math.abs(dx)) { pointer = null; return }
      if (Math.abs(dx) < 9 || Math.abs(dx) < Math.abs(dy) * 1.2) return
      if ((origin.value === pages[0] && dx > 0) || (origin.value === pages.at(-1) && dx < 0)) { pointer = null; return }
      dragging.value = true
      try { root.value?.setPointerCapture(event.pointerId) } catch { /* Capture is optional in browser previews. */ }
    }
    event.preventDefault(); event.stopPropagation()
    offset.value = Math.max(pages.indexOf(origin.value) === pages.length - 1 ? 0 : -width.value, Math.min(pages.indexOf(origin.value) === 0 ? 0 : width.value, dx))
  }
  function end(event: PointerEvent, cancelled = false): void {
    if (event.pointerId !== pointer) return
    pointer = null
    if (dragging.value) event.stopPropagation()
    finish(cancelled)
  }
  function native(event: NativeSwipeEvent): void {
    if (!Number.isFinite(event.dx)) return
    if (event.phase === 'start') {
      if (page.value !== 'session' || animating.value || !canStart()) return
      start(); dragging.value = nativeDragging.value = true
    }
    if (!nativeDragging.value) return
    offset.value = Math.min(width.value, Math.max(0, event.dx))
    if (event.phase === 'end' || event.phase === 'cancel') finish(event.phase === 'cancel')
  }
  function slideTo(destination: Workspace): void {
    if (page.value === destination || animating.value || dragging.value) return
    start(); offset.value = 0; dragging.value = true
    void nextTick(() => {
      if (!dragging.value) return
      void root.value?.offsetWidth
      settle(true, destination)
    })
  }
  function guardClick(event: MouseEvent): void {
    if (performance.now() < clickUntil) { event.preventDefault(); event.stopPropagation() }
  }
  onBeforeUnmount(cancel)
  return { dragging, animating, nativeDragging, visible, progress, indicatorStyle, transform, begin, move, end, native, slideTo, guardClick, cancel }
}
