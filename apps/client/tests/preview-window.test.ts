import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import PreviewZoom from '../src/shared/components/PreviewZoom.vue'
import ZoomViewport from '../src/shared/components/ZoomViewport.vue'
import type { TabState } from '../src/shared/state/controller'
import { boundPanel, resizeEdges, resizePanel } from '../src/shared/components/preview-window'

const tab: TabState = {
  key: 'test', targetId: 'test', hostId: 'host', hostLabel: 'Host', label: 'Shell',
  status: 'live', errorMsg: '', notice: '', sessionId: 'test', remoteSession: 'shell', runtime: true,
  runtimeAgent: false, exitCode: null, desktopAvailable: true, desktopEnabled: true, desktopBusy: false,
  desktopMsg: '', desktopError: '', previewVisible: true, shot: null, shotError: '', captureGen: 0,
  term: null, wantDesktop: true, closed: false, writeBuf: [], writing: false, writeDropped: false,
  pendingSize: null, resizing: false, lastSize: null, connectionGen: 0
}
const area = { width: 1000, height: 700 }, minimum = { width: 400, height: 250 }
const initial = { x: 200, y: 150, width: 500, height: 350 }
let resize: () => void
let width = 1000, height = 700
const wrappers: VueWrapper[] = []
beforeEach(() => {
  width = 1000; height = 700
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { resize = callback } observe() {} disconnect() {} })
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => new DOMRect(0, 0, width, height))
  vi.stubGlobal('PointerEvent', MouseEvent)
  HTMLElement.prototype.setPointerCapture = vi.fn()
})
afterEach(() => { for (const wrapper of wrappers.splice(0)) wrapper.unmount(); vi.restoreAllMocks(); vi.unstubAllGlobals() })
async function setup(mode: 'windows' | 'android' = 'windows') {
  const wrapper = mount(PreviewZoom, { props: { tab, mode, screenKey: 'screen-A' }, global: { stubs: { ZoomViewport: { template: '<div class="image-fixture" />' } } } })
  wrappers.push(wrapper)
  return wrapper
}
async function pointer(wrapper: VueWrapper, selector: string, type: string, x = 100, y = 100, extra = {}) {
  const event = new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0 })
  Object.defineProperties(event, { pointerId: { value: 1, configurable: true }, pointerType: { value: 'mouse', configurable: true }, isPrimary: { value: true, configurable: true } })
  for (const [key, value] of Object.entries(extra)) Object.defineProperty(event, key, { value })
  wrapper.get(selector).element.dispatchEvent(event)
  await nextTick()
}
async function click(wrapper: VueWrapper) { await pointer(wrapper, '.image-fixture', 'pointerdown'); await pointer(wrapper, '.image-fixture', 'pointerup') }
function rect(wrapper: VueWrapper) { return wrapper.get('.zoom-panel').attributes('style') }

describe('floating preview bounds', () => {
  it('keeps all eight resize directions inside the app and at least the opening dimensions', () => {
    for (const edge of resizeEdges) for (const delta of [{ x: 5000, y: 5000 }, { x: -5000, y: -5000 }]) {
      const result = resizePanel(initial, edge, delta, minimum, area)
      expect(result.width).toBeGreaterThanOrEqual(minimum.width)
      expect(result.height).toBeGreaterThanOrEqual(minimum.height)
      expect(result.x).toBeGreaterThanOrEqual(0); expect(result.y).toBeGreaterThanOrEqual(0)
      expect(result.x + result.width).toBeLessThanOrEqual(area.width)
      expect(result.y + result.height).toBeLessThanOrEqual(area.height)
      if (edge.includes('w')) expect(result.x + result.width).toBe(initial.x + initial.width)
      if (edge.includes('n')) expect(result.y + result.height).toBe(initial.y + initial.height)
    }
  })
  it('temporarily fits a smaller app and recovers the saved dimensions when space returns', () => {
    expect(boundPanel(initial, minimum, { width: 180, height: 120 })).toEqual({ x: 0, y: 0, width: 180, height: 120 })
    expect(boundPanel(initial, minimum, area)).toEqual(initial)
  })
})
it('single left click maximizes only inside the app and restores the last moved/resized rectangle', async () => {
  const wrapper = await setup()
  await pointer(wrapper, '.zoom-title', 'pointerdown')
  await pointer(wrapper, '.zoom-title', 'pointermove', 200, 150)
  await pointer(wrapper, '.zoom-title', 'pointerup', 200, 150)
  await pointer(wrapper, '[data-resize="se"]', 'pointerdown')
  await pointer(wrapper, '[data-resize="se"]', 'pointermove', 250, 180)
  await pointer(wrapper, '[data-resize="se"]', 'pointerup', 250, 180)
  const saved = rect(wrapper)
  await click(wrapper)
  expect(wrapper.get('.zoom-panel').classes()).toContain('maximized')
  expect(rect(wrapper)).toContain('width: 1000px'); expect(rect(wrapper)).toContain('height: 700px')
  width = 800; height = 600; resize(); await nextTick()
  expect(rect(wrapper)).toContain('width: 800px')
  await click(wrapper)
  expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
  expect(rect(wrapper)).toBe(saved)
})
it('does not confuse image pan, title move, resize, wheel, cancellation or multiple pointers with a click', async () => {
  const wrapper = await setup()
  for (const selector of ['.image-fixture', '.zoom-title', '[data-resize="se"]']) {
    await pointer(wrapper, selector, 'pointerdown')
    await pointer(wrapper, selector, 'pointermove', 101, 100)
    await pointer(wrapper, selector, 'pointerup', 101, 100)
    expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
  }
  for (const cancel of ['wheel', 'pointercancel', 'lostpointercapture']) {
    await pointer(wrapper, '.image-fixture', 'pointerdown')
    await wrapper.get('.image-fixture').trigger(cancel)
    await pointer(wrapper, '.image-fixture', 'pointerup')
    expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
  }
  await pointer(wrapper, '.image-fixture', 'pointerdown')
  await pointer(wrapper, '.image-fixture', 'pointerdown', 100, 100, { pointerId: 2, pointerType: 'touch', isPrimary: false })
  await pointer(wrapper, '.image-fixture', 'pointerup')
  expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
  await pointer(wrapper, '.image-fixture', 'pointerdown', 100, 100, { button: 2 })
  await pointer(wrapper, '.image-fixture', 'pointerup', 100, 100, { button: 2 })
  expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
})
it('close still closes without toggling and Android has no floating panel or resize handles', async () => {
  const wrapper = await setup()
  await pointer(wrapper, '.zoom-close', 'pointerdown'); await pointer(wrapper, '.zoom-close', 'pointerup')
  await wrapper.get('.zoom-close').trigger('click')
  expect(wrapper.emitted('close')).toHaveLength(1)
  expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
  const mobile = await setup('android')
  expect(mobile.find('.mobile-zoom').exists()).toBe(true)
  expect(mobile.find('.zoom-panel').exists()).toBe(false)
  expect(mobile.find('.zoom-resize').exists()).toBe(false)
})
it('keeps user size through screenshot dimension refreshes and resets only for another Screen', async () => {
  const wrapper = await setup()
  const image = wrapper.getComponent(ZoomViewport)
  image.vm.$emit('dimensions', { width: 1920, height: 1080 }); await nextTick()
  await pointer(wrapper, '[data-resize="se"]', 'pointerdown')
  await pointer(wrapper, '[data-resize="se"]', 'pointermove', 180, 160)
  await pointer(wrapper, '[data-resize="se"]', 'pointerup', 180, 160)
  const saved = rect(wrapper)
  image.vm.$emit('dimensions', { width: 1080, height: 1920 }); await nextTick()
  expect(rect(wrapper)).toBe(saved)
  await wrapper.setProps({ screenKey: 'screen-B' })
  image.vm.$emit('dimensions', { width: 1080, height: 1920 }); await nextTick()
  expect(rect(wrapper)).not.toBe(saved)
})
it('ignores mouse clicks while an earlier touch is held and recovers after up or cancellation', async () => {
  const wrapper = await setup()
  for (const finish of ['pointerup', 'pointercancel']) {
    await pointer(wrapper, '.image-fixture', 'pointerdown', 100, 100, { pointerId: 2, pointerType: 'touch' })
    await click(wrapper)
    expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
    await pointer(wrapper, '.image-fixture', finish, 100, 100, { pointerId: 2, pointerType: 'touch' })
    await click(wrapper)
    expect(wrapper.get('.zoom-panel').classes()).toContain('maximized')
    await click(wrapper)
    expect(wrapper.get('.zoom-panel').classes()).not.toContain('maximized')
  }
})
it('retains the saved floating position when maximized while the app is temporarily smaller', async () => {
  const wrapper = await setup()
  await pointer(wrapper, '.zoom-title', 'pointerdown')
  await pointer(wrapper, '.zoom-title', 'pointermove', 700, 500)
  await pointer(wrapper, '.zoom-title', 'pointerup', 700, 500)
  const saved = rect(wrapper)
  width = 320; height = 220; resize(); await nextTick()
  await click(wrapper)
  expect(wrapper.get('.zoom-panel').classes()).toContain('maximized')
  width = 1000; height = 700; resize(); await nextTick()
  await click(wrapper)
  expect(rect(wrapper)).toBe(saved)
})