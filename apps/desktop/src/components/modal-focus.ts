import { nextTick, onBeforeUnmount, onMounted, type Ref } from 'vue'
export function useModalFocus(root: Ref<HTMLElement | null>, close: () => void): void {
  let previous: HTMLElement | null = null
  const controls = () => Array.from(root.value?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled)') ?? [])
  function key(event: KeyboardEvent): void {
    if (!root.value) return
    if (event.key === 'Escape') { event.preventDefault(); close(); return }
    if (event.key !== 'Tab') return
    const items = controls(), first = items[0], last = items.at(-1)
    if (!first || !last) { event.preventDefault(); root.value.focus(); return }
    if (!root.value.contains(document.activeElement) || (event.shiftKey && document.activeElement === first) || (!event.shiftKey && document.activeElement === last)) { event.preventDefault(); (event.shiftKey ? last : first).focus() }
  }
  onMounted(async () => { previous = document.activeElement instanceof HTMLElement ? document.activeElement : null; document.addEventListener('keydown', key); await nextTick(); (controls()[0] ?? root.value)?.focus() })
  onBeforeUnmount(() => { document.removeEventListener('keydown', key); if (previous?.isConnected) previous.focus() })
}
