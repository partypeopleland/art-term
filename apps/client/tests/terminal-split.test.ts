import { mount, type VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from '../src/desktop/App.vue'
import type { AppController } from '../src/shared/state/controller'
import {
  clampPaneRatio,
  findPaneLeaf,
  paneConnTabs,
  paneLeaves,
  removePaneLeaf,
  replacePaneLeaf,
  splitAxis,
  splitPaneLeaf,
  wrapSplit,
  type PaneNode
} from '../src/shared/terminal/split'
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
  const other = makeTarget('other', 'host', 'host', true, false)
  const transport = new FakeTransport([makeProfile('host', 'My computer')])
  transport.runtimeSessionsData.set('host', makeHost('host', [target, other]))
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
  return { wrapper, controller, transport, terms, tab: controller.tabs[0], other }
}

const conn = (id: string) => ({ kind: 'conn' as const, id, tab: null as never })
const empty = (id: string) => ({ kind: 'empty' as const, id })

describe('pane tree helpers', () => {
  it('maps directions to axes and placement', () => {
    expect(splitAxis('left')).toBe('row')
    expect(splitAxis('right')).toBe('row')
    expect(splitAxis('up')).toBe('column')
    expect(splitAxis('down')).toBe('column')
    const inner = conn('a')
    const added = empty('b')
    expect(wrapSplit('right', inner, added, 's').a).toBe(inner)
    expect(wrapSplit('left', inner, added, 's').a).toBe(added)
    expect(wrapSplit('down', inner, added, 's').b).toBe(added)
    expect(wrapSplit('up', inner, added, 's').a).toBe(added)
  })

  it('splits, replaces and removes leaves while collapsing single-child splits', () => {
    const root: PaneNode = wrapSplit('right', conn('a'), empty('b'), 's1')
    const split = splitPaneLeaf(root, 'b', 'down', empty('c'), 's2') as Extract<PaneNode, { kind: 'split' }>
    expect(split.b.kind).toBe('split')
    expect(findPaneLeaf(split, 'c')?.kind).toBe('empty')

    const replaced = replacePaneLeaf(split, 'c', conn('d'))
    expect(findPaneLeaf(replaced, 'd')?.kind).toBe('conn')
    expect(findPaneLeaf(replaced, 'c')).toBeNull()

    const collapsed = removePaneLeaf(replaced, 'd')
    expect(collapsed?.kind).toBe('split')
    const single = removePaneLeaf(collapsed!, 'b')
    expect(single).toEqual(conn('a'))
    expect(removePaneLeaf(single!, 'a')).toBeNull()
    expect(clampPaneRatio(0.01)).toBe(0.15)
    expect(clampPaneRatio(0.99)).toBe(0.85)
    expect(clampPaneRatio(0.4)).toBe(0.4)
  })
})

describe('Windows independent split panes', () => {
  it('splits into an empty pane and connects it to its own session', async () => {
    const { wrapper, controller, transport, terms, tab, other } = await setup()
    const sid = tab.sessionId!
    const liveTerms = () => terms.filter((t) => !t.disposed)
    expect(controller.splitPane(tab, tab.key, 'right')).toBe(true)
    await flush()
    expect(tab.paneRoot?.kind).toBe('split')
    // The primary pane remounts inside the split tree; the stale handle is released.
    expect(liveTerms()).toHaveLength(1)
    expect(wrapper.find('[data-testid="pane-pick-connection"]').exists()).toBe(true)

    const emptyId = paneLeaves(tab.paneRoot!).find((l) => l.kind === 'empty')!.id
    controller.openPaneTarget(tab, emptyId, makeHost('host', [other]), other)
    await flush()
    const child = controller.tabs.find((t) => t.paneOf === tab.key)!
    expect(child).toBeTruthy()
    expect(child.targetId).toBe('other')
    expect(child.status).toBe('live')
    expect(child.sessionId).not.toBe(sid)
    expect(transport.openCalls.map((c) => c.targetId)).toContain('other')
    expect(liveTerms()).toHaveLength(2)
    expect(paneConnTabs(tab.paneRoot!)).toEqual([tab, child])
    const childTerm = terms.find((t) => t === (child.term as unknown as SplitTerm))!

    // Independent input: each pane writes only to its own session.
    ;(tab.term as unknown as SplitTerm).emitData('primary-input')
    childTerm.emitData('pane-input')
    await flush()
    expect(transport.sessions.get(sid)!.writes.flat()).toEqual(Array.from(new TextEncoder().encode('primary-input')))
    expect(transport.sessions.get(child.sessionId!)!.writes.flat()).toEqual(Array.from(new TextEncoder().encode('pane-input')))
  })

  it('closing one pane ends only that pane connection', async () => {
    const { controller, transport, terms, tab, other } = await setup()
    const sid = tab.sessionId!
    controller.splitPane(tab, tab.key, 'down')
    const emptyId = paneLeaves(tab.paneRoot!).find((l) => l.kind === 'empty')!.id
    controller.openPaneTarget(tab, emptyId, makeHost('host', [other]), other)
    await flush()
    const child = controller.tabs.find((t) => t.paneOf === tab.key)!
    const childSid = child.sessionId!
    const childTermHandle = child.term as unknown as SplitTerm

    await controller.closeTab(child)
    await flush()
    expect(controller.tabs.find((t) => t.key === child.key)).toBeUndefined()
    expect(transport.closeCalls).toContain(childSid)
    expect(transport.closeCalls).not.toContain(sid)
    expect(tab.status).toBe('live')
    expect(tab.paneRoot).toBeNull()
    // The child's terminal is disposed; the primary pane's live handle survives
    // the collapse remount.
    expect(childTermHandle.disposed).toBe(true)
    expect(tab.term).toBeTruthy()
    expect((tab.term as unknown as SplitTerm).disposed).toBe(false)
  })

  it('keeps the read-only mirror leaf working next to independent panes', async () => {
    const { controller, transport, terms, tab } = await setup()
    const sid = tab.sessionId!
    terms[0].snapshotText = 'SNAPSHOT'
    transport.feed(sid, { bytes: [72, 105], exitCode: null })
    await flush()

    expect(controller.mirrorPane(tab, tab.key, 'down')).toBe(true)
    await flush()
    const liveTerms = () => terms.filter((t) => !t.disposed)
    expect(liveTerms()).toHaveLength(2)
    const mirrorTerm = tab.mirrorTerm as unknown as SplitTerm
    const primaryTerm = tab.term as unknown as SplitTerm
    transport.feed(sid, { bytes: [89, 111], exitCode: null })
    await flush()
    expect(primaryTerm.text()).toContain('Yo')
    expect(mirrorTerm.text()).toContain('Yo')

    mirrorTerm.emitData('mirror-input')
    await flush()
    expect(transport.sessions.get(sid)!.writes.flat()).toEqual([])

    const mirrorId = `mirror:${tab.key}`
    controller.closePaneLeaf(tab, mirrorId)
    await flush()
    expect(tab.paneRoot).toBeNull()
    expect(transport.closeCalls).toEqual([])
    expect(mirrorTerm.disposed).toBe(true)
    expect(tab.status).toBe('live')
  })

  it('does not close the primary conn pane as a single pane', async () => {
    const { controller, transport, tab, other } = await setup()
    controller.splitPane(tab, tab.key, 'right')
    const emptyId = paneLeaves(tab.paneRoot!).find((l) => l.kind === 'empty')!.id
    controller.openPaneTarget(tab, emptyId, makeHost('host', [other]), other)
    await flush()
    controller.closePaneLeaf(tab, tab.key)
    await flush()
    expect(tab.closed).toBe(false)
    expect(tab.status).toBe('live')
    expect(controller.tabs.filter((t) => t.paneOf === tab.key)).toHaveLength(1)
    expect(transport.closeCalls).toEqual([])
  })

  it('cycles pane focus with keyboard order and closes the focused child pane', async () => {
    const { controller, transport, tab, other } = await setup()
    controller.splitPane(tab, tab.key, 'right')
    const emptyId = paneLeaves(tab.paneRoot!).find((l) => l.kind === 'empty')!.id
    controller.openPaneTarget(tab, emptyId, makeHost('host', [other]), other)
    await flush()
    const child = controller.tabs.find((t) => t.paneOf === tab.key)!

    expect(tab.focusedLeafId).toBe(child.key)
    controller.cyclePaneFocus(tab, -1)
    expect(tab.focusedLeafId).toBe(tab.key)
    controller.cyclePaneFocus(tab, 1)
    expect(tab.focusedLeafId).toBe(child.key)
    expect(controller.focusedConnTab(tab)).toBe(child)

    const childSid = child.sessionId!
    const closeEvent = new KeyboardEvent('keydown', { code: 'KeyW', ctrlKey: true, shiftKey: true })
    expect(controller.handleKey(closeEvent)).toBe(true)
    await flush()
    expect(controller.tabs.find((t) => t.key === child.key)).toBeUndefined()
    expect(transport.closeCalls).toContain(childSid)
  })

  it('unsplitPanes closes every extra pane but keeps the primary connection', async () => {
    const { controller, transport, tab, other } = await setup()
    const sid = tab.sessionId!
    controller.splitPane(tab, tab.key, 'right')
    controller.splitPane(tab, tab.key, 'down')
    const empties = paneLeaves(tab.paneRoot!).filter((l) => l.kind === 'empty')
    for (const leaf of empties) {
      controller.openPaneTarget(tab, leaf.id, makeHost('host', [other]), other)
    }
    await flush()
    const children = controller.tabs.filter((t) => t.paneOf === tab.key)
    expect(children.length).toBeGreaterThanOrEqual(2)

    controller.unsplitPanes(tab)
    await flush()
    expect(controller.tabs.filter((t) => t.paneOf === tab.key)).toHaveLength(0)
    expect(tab.paneRoot).toBeNull()
    expect(tab.status).toBe('live')
    expect(transport.closeCalls).not.toContain(sid)
    for (const child of children) expect(child.closed).toBe(true)
  })
})
