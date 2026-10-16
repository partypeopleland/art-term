import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick, ref } from 'vue'
import { useWorkspacePager, type Workspace } from '../src/mobile/pager'

const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => { wrappers.splice(0).forEach(wrapper => wrapper.unmount()); vi.useRealTimers() })
function setup(initial: Workspace = 'list') {
  const page = ref(initial)
  const enabled = ref(true)
  let pager!: ReturnType<typeof useWorkspacePager>
  const activate = vi.fn((destination: Workspace) => { page.value = destination })
  const wrapper = mount({ setup() {
    const root = ref<HTMLElement | null>(null)
    pager = useWorkspacePager(root, page, activate, () => enabled.value)
    return { root }
  }, template: '<div ref="root"><button>Tap</button></div>' })
  wrappers.push(wrapper)
  Object.defineProperty(wrapper.element, 'clientWidth', { value: 400 })
  const event = (x: number, y = 100) => ({ target: wrapper.element, isPrimary: true, button: 0, pointerId: 1, clientX: x, clientY: y, preventDefault: vi.fn(), stopPropagation: vi.fn() }) as unknown as PointerEvent
  return { pager, page, enabled, activate, event }
}

describe('shared workspace drag', () => {
  it('tracks the visible fraction and cancels a short drag without a page switch', async () => {
    vi.useFakeTimers()
    const { pager, page, event, activate } = setup()
    pager.begin(event(300)); pager.move(event(240))
    expect(pager.progress.value).toBe(.15)
    expect(pager.transform('list')).toBe('translateX(-60px)')
    expect(pager.transform('session')).toBe('translateX(340px)')
    await vi.advanceTimersByTimeAsync(400)
    pager.end(event(240))
    await vi.advanceTimersByTimeAsync(200)
    expect(page.value).toBe('list'); expect(activate).not.toHaveBeenCalled()
  })
  it('slides half way with the indicator, commits once and suppresses the drag click', async () => {
    vi.useFakeTimers()
    const { pager, page, event, activate } = setup()
    pager.begin(event(300)); pager.move(event(100))
    expect(pager.progress.value).toBe(.5)
    expect(pager.indicatorStyle.value.transform).toBe('translateX(50%)')
    pager.end(event(100))
    const click = { preventDefault: vi.fn(), stopPropagation: vi.fn() } as unknown as MouseEvent
    pager.guardClick(click); expect(click.preventDefault).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(200)
    expect(page.value).toBe('session'); expect(activate).toHaveBeenCalledOnce()
    expect(pager.progress.value).toBe(1)
  })
  it('leaves vertical terminal scrolling and invalid direction gestures alone', () => {
    const { pager, event } = setup('session')
    pager.begin(event(100)); pager.move(event(102, 180))
    expect(pager.visible.value).toBe(false)
    pager.begin(event(300)); pager.move(event(100))
    expect(pager.visible.value).toBe(false)
  })
  it('leaves horizontally scrolling key and terminal-tab bars to their own controls', () => {
    const { pager, event } = setup('session')
    const keybar = document.createElement('div'); keybar.className = 'keybar'
    const button = document.createElement('button'); keybar.append(button)
    const start = event(100)
    Object.defineProperty(start, 'target', { value: button })
    pager.begin(start); pager.move(event(300))
    expect(pager.visible.value).toBe(false)
  })
  it('accepts native Android right drags and cancels without navigation on interruption', async () => {
    vi.useFakeTimers()
    const { pager, page } = setup('session')
    pager.native({ phase: 'start', dx: 20 }); pager.native({ phase: 'move', dx: 200 })
    expect(pager.nativeDragging.value).toBe(true)
    expect(pager.progress.value).toBe(.5)
    pager.native({ phase: 'cancel', dx: 200 })
    await vi.advanceTimersByTimeAsync(200)
    expect(page.value).toBe('session')
    pager.native({ phase: 'start', dx: 20 }); pager.native({ phase: 'end', dx: 200 })
    await vi.advanceTimersByTimeAsync(200)
    expect(page.value).toBe('list')
  })
  it('ignores native gestures while a full-page settings/voice overlay owns input', () => {
    const { pager, enabled } = setup('session')
    enabled.value = false
    pager.native({ phase: 'start', dx: 100 })
    pager.native({ phase: 'end', dx: 200 })
    expect(pager.visible.value).toBe(false)
  })
  it('cancels a pending animated navigation on disposal or background', async () => {
    vi.useFakeTimers()
    const { pager, activate } = setup()
    pager.slideTo('session'); await nextTick()
    pager.cancel(); await vi.advanceTimersByTimeAsync(250)
    expect(activate).not.toHaveBeenCalled()
  })
})
