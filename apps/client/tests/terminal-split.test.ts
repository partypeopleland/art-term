import { mount, type VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from '../src/desktop/App.vue'
import type { AppController } from '../src/shared/state/controller'
import { splitLayout, splitPaneOrder, type SplitDirection } from '../src/shared/terminal/split'
import { FakeTerm, FakeTransport, makeHost, makeProfile, makeTarget } from './fake'

vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })

const wrappers: VueWrapper[] = []
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  document.body.replaceChildren()
})
async function flush(times = 60): Promise<void> {
  for (let i = 0; i < times; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

class SplitTerm extends FakeTerm {
  snapshotText = ''
  screenSnapshot(): string {
    return this.snapshotText
  }
}

async function setup() {
  const target = makeTarget('shell', 'host', 'host', true, false)
  const transport = new FakeTransport([makeProfile('host', 'My computer')])
  const terms: SplitTerm[] = []
  const wrapper = mount(App, {
    props: {
      transport,
      terminalFactory: () => {
        const term = new SplitTerm()
        terms.push(term)
        return term
      },
      controllerOptions: { idlePollMs: 0, autoCaptureMs: 30_000 }
    },
    attachTo: document.body
  })
  wrappers.push(wrapper)
  await flush()
  const controller = (wrapper.vm as unknown as { controller: AppController }).controller
  controller.openTarget({ dto: makeHost('host', [target]), expanded: true }, target, false)
  await flush()
  return { wrapper, controller, transport, terms, tab: controller.tabs[0] }
}

function paneOrder(wrapper: VueWrapper): string[] {
  return wrapper.findAll('.tab-view .pane').map((pane) => pane.attributes('data-pane') ?? '')
}

describe('Windows terminal split panes', () => {
  it('maps the four directions to the correct axis and mirror position', () => {
    expect(splitLayout('up')).toEqual({ axis: 'column', mirrorFirst: true })
    expect(splitLayout('down')).toEqual({ axis: 'column', mirrorFirst: false })
    expect(splitLayout('left')).toEqual({ axis: 'row', mirrorFirst: true })
    expect(splitLayout('right')).toEqual({ axis: 'row', mirrorFirst: false })
    expect(splitPaneOrder('up')).toEqual(['mirror', 'divider', 'primary'])
    expect(splitPaneOrder('down')).toEqual(['primary', 'divider', 'mirror'])
    expect(splitPaneOrder('left')).toEqual(['mirror', 'divider', 'primary'])
    expect(splitPaneOrder('right')).toEqual(['primary', 'divider', 'mirror'])
  })

  it('renders primary and mirror panes in the requested direction', async () => {
    const { wrapper, controller, tab } = await setup()
    for (const direction of ['up', 'down', 'left', 'right'] as SplitDirection[]) {
      expect(controller.splitTab(tab, direction)).toBe(true)
      await flush()
      const layout = splitLayout(direction)
      expect(paneOrder(wrapper)).toEqual(layout.mirrorFirst ? ['mirror', 'primary'] : ['primary', 'mirror'])
      expect(wrapper.find('.tab-view').classes()).toContain(layout.axis === 'column' ? 'split-column' : 'split-row')
    }
  })

  it('mirrors the same session read-only while the primary pane keeps input', async () => {
    const { wrapper, controller, transport, terms, tab } = await setup()
    const sid = tab.sessionId!
    terms[0].snapshotText = 'SNAPSHOT'
    transport.feed(sid, { bytes: [72, 105], exitCode: null })
    await flush()

    expect(controller.splitTab(tab, 'down')).toBe(true)
    await flush()
    expect(terms).toHaveLength(2)
    expect(terms[0].text()).toContain('Hi')
    expect(terms[1].text()).toContain('SNAPSHOT')

    transport.feed(sid, { bytes: [89, 111], exitCode: null })
    await flush()
    expect(terms[0].text()).toContain('Yo')
    expect(terms[1].text()).toContain('Yo')

    terms[1].emitData('mirror-input')
    terms[0].emitData('primary-input')
    await flush()
    const written = transport.sessions.get(sid)!.writes.flat()
    expect(written).toEqual(Array.from(new TextEncoder().encode('primary-input')))

    await wrapper.find('[data-pane="mirror"] .pane-close').trigger('click')
    await flush()
    expect(tab.split).toBeNull()
    expect(tab.status).toBe('live')
    expect(transport.closeCalls).toEqual([])
    expect(terms[1].disposed).toBe(true)
    expect(terms[0].disposed).toBe(false)

    transport.feed(sid, { bytes: [33], exitCode: null })
    await flush()
    expect(terms[0].text()).toContain('!')
  })

  it('closing the input pane keeps the mirror and connection alive', async () => {
    const { wrapper, controller, transport, terms, tab } = await setup()
    const sid = tab.sessionId!
    expect(controller.splitTab(tab, 'right')).toBe(true)
    await flush()
    expect(terms).toHaveLength(2)

    await wrapper.find('[data-pane="primary"] .pane-close').trigger('click')
    await flush()
    expect(tab.inputPaneHidden).toBe(true)
    expect(tab.split).toBe('right')
    expect(tab.status).toBe('live')
    expect(transport.closeCalls).toEqual([])
    expect(wrapper.find('[data-pane="primary"]').isVisible()).toBe(false)
    expect(wrapper.find('[data-pane="mirror"]').isVisible()).toBe(true)

    transport.feed(sid, { bytes: [65], exitCode: null })
    await flush()
    expect(terms[1].text()).toContain('A')

    await wrapper.find('[data-pane="mirror"] .pane-close').trigger('click')
    await flush()
    expect(tab.split).toBeNull()
    expect(tab.inputPaneHidden).toBe(false)
    expect(tab.status).toBe('live')
    expect(transport.closeCalls).toEqual([])
    expect(terms[1].disposed).toBe(true)
    expect(wrapper.find('[data-pane="primary"]').isVisible()).toBe(true)

    transport.feed(sid, { bytes: [66], exitCode: null })
    await flush()
    expect(terms[0].text()).toContain('B')
  })
})
