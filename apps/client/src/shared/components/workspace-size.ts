import { computed, onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

export function useWorkspaceSize(workspace: Ref<HTMLElement | null>) {
  const width = ref(360), height = ref(200)
  const bounds = ref({ width: 1000, height: 700 })
  const maxWidth = computed(() => Math.max(160, bounds.value.width - 320))
  const maxHeight = computed(() => Math.max(100, bounds.value.height - 180))
  const sidebarWidth = computed(() => Math.min(width.value, maxWidth.value))
  const previewHeight = computed(() => Math.min(height.value, maxHeight.value))
  let observer: ResizeObserver | undefined
  let drag: { axis: 'width' | 'height'; pointer: number; target: HTMLElement; origin: number; size: number } | undefined

  function end() {
    if (drag?.target.hasPointerCapture?.(drag.pointer)) drag.target.releasePointerCapture(drag.pointer)
    drag = undefined
  }
  function change(axis: 'width' | 'height', value: number) {
    if (axis === 'width') width.value = Math.max(160, Math.min(maxWidth.value, value))
    else height.value = Math.max(100, Math.min(maxHeight.value, value))
  }
  function start(event: PointerEvent, axis: 'width' | 'height') {
    if (event.button !== 0 || !(event.currentTarget instanceof HTMLElement)) return
    end()
    const target = event.currentTarget
    drag = { axis, pointer: event.pointerId, target, origin: axis === 'width' ? event.clientX : event.clientY,
      size: axis === 'width' ? sidebarWidth.value : previewHeight.value }
    target.setPointerCapture(event.pointerId)
    event.preventDefault()
  }
  function move(event: PointerEvent) {
    if (!drag || drag.pointer !== event.pointerId) return
    const delta = (drag.axis === 'width' ? event.clientX : event.clientY) - drag.origin
    change(drag.axis, drag.size + (drag.axis === 'height' ? -delta : delta))
    event.preventDefault()
  }
  function release(event: PointerEvent) { if (event.pointerId === drag?.pointer) end() }
  function key(event: KeyboardEvent, axis: 'width' | 'height') {
    const delta = axis === 'width' ? { ArrowLeft: -16, ArrowRight: 16 } : { ArrowUp: 16, ArrowDown: -16 }
    const amount = delta[event.key as keyof typeof delta]
    if (amount === undefined) return
    event.preventDefault(); event.stopPropagation()
    change(axis, (axis === 'width' ? sidebarWidth.value : previewHeight.value) + amount)
  }
  onMounted(() => {
    observer = new ResizeObserver(() => {
      const node = workspace.value
      if (node && node.clientWidth && node.clientHeight) bounds.value = { width: node.clientWidth, height: node.clientHeight }
    })
    if (workspace.value) observer.observe(workspace.value)
    window.addEventListener('blur', end)
  })
  onBeforeUnmount(() => { end(); observer?.disconnect(); window.removeEventListener('blur', end) })
  return { sidebarWidth, previewHeight, maxWidth, maxHeight, start, move, release, end, key }
}
