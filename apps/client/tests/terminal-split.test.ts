import { mount, type VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from '../src/desktop/App.vue'
import type { AppController, TabState } from '../src/shared/state/controller'
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
  type PaneNode,
  type SplitDirection
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
  const transport = new FakeTransport([makeProfile('host', 'My computer')])
  transport.runtimeSessionsData.set('host', makeHost('host', [target]))
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

/** Split and let the auto-created pane connection finish mounting. */
async function split(
  controller: AppController,
  tab: TabState,
  direction: SplitDirection = 'right',
  leafId = tab.key
): Promise<boolean> {
  const ok = await controller.splitPane(tab, leafId, direction)
  await flush()
  return ok
}

function paneChild(controller: AppController, tab: TabState): TabState {
  return controller.tabs.find((t) => t.paneOf === tab.key)!
}

const conn = (id: string) => ({ kind: 'conn' as const, id, tab: null as never })

describe('pane tree helpers', () => {
  it('maps directions to axes and placement', () => {
    expect(splitAxis('left')).toBe('row')
    expect(splitAxis('right')).toBe('row')
    expect(splitAxis('up')).toBe('column')
    expect(splitAxis('down')).toBe('column')
    const inner = conn('a')
    const added = conn('b')
    expect(wrapSplit('right', inner, added, 's').a).toBe(inner)
    expect(wrapSplit('left', inner, added, 's').a).toBe(added)
    expect(wrapSplit('down', inner, added, 's').b).toBe(added)
    expect(wrapSplit('up', inner, added, 's').a).toBe(added)
  })

  it('splits, replaces and removes leaves while collapsing single-child splits', () => {
    const root: PaneNode = wrapSplit('right', conn('a'), conn('b'), 's1')
    const split = splitPaneLeaf(root, 'b', 'down', conn('c'), 's2') as Extract<PaneNode, { kind: 'split' }>
    expect(split.b.kind).toBe('split')
    expect(findPaneLeaf(split, 'c')?.kind).toBe('conn')

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
  it('splits into a new connection created on the same host, with no picker', async () => {
    const { wrapper, controller, transport, terms, tab } = await setup()
    const sid = tab.sessionId!
    const liveTerms = () => terms.filter((t) => !t.disposed)
    expect(await split(controller, tab, 'right')).toBe(true)
    expect(tab.paneRoot?.kind).toBe('split')
    // The primary pane remounts inside the split tree; the stale handle is released.
    // The new pane is already a live conn leaf — no empty pane, no picker.
    expect(wrapper.find('[data-testid="pane-connection-picker"]').exists()).toBe(false)
    const child = paneChild(controller, tab)
    expect(child).toBeTruthy()
    expect(child.status).toBe('live')
    expect(child.sessionId).not.toBe(sid)
    expect(child.targetId).toMatch(/^history-/)
    expect(transport.historyCalls).toEqual([{ id: 'host', expectedRevision: 1 }])
    expect(transport.openCalls.map((c) => c.targetId)).toContain(child.targetId)
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

  it('adds no pane and reports a notice when session creation fails', async () => {
    const { controller, transport, tab } = await setup()
    transport.historyError = new Error('create failed')
    expect(await split(controller, tab, 'right')).toBe(false)
    expect(tab.paneRoot).toBeNull()
    expect(controller.tabs.filter((t) => t.paneOf === tab.key)).toHaveLength(0)
    expect(controller.uiNotice.value).toBeTruthy()
    expect(transport.openCalls).toHaveLength(1)
  })

  it('refuses to split when the transport cannot create history sessions', async () => {
    const { controller, transport, tab } = await setup()
    ;(transport as { runtimeCreateHistorySession?: unknown }).runtimeCreateHistorySession = undefined
    expect(await split(controller, tab, 'right')).toBe(false)
    expect(tab.paneRoot).toBeNull()
    expect(controller.tabs.filter((t) => t.paneOf === tab.key)).toHaveLength(0)
  })

  it('closing one pane ends only that pane connection', async () => {
    const { controller, transport, terms, tab } = await setup()
    const sid = tab.sessionId!
    await split(controller, tab, 'down')
    const child = paneChild(controller, tab)
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

  it('closing the primary pane promotes a surviving child connection', async () => {
    const { controller, transport, terms, tab } = await setup()
    const sid = tab.sessionId!
    await split(controller, tab, 'right')
    await split(controller, tab, 'down')
    const children = controller.tabs.filter((t) => t.paneOf === tab.key)
    expect(children).toHaveLength(2)
    const childSids = children.map((c) => c.sessionId)
    const primaryTerm = tab.term as unknown as SplitTerm

    await controller.closePaneLeaf(tab, tab.key)
    await flush()

    // Only the primary connection ended; both child sessions survive.
    expect(tab.closed).toBe(true)
    expect(controller.tabs.find((t) => t.key === tab.key)).toBeUndefined()
    expect(transport.closeCalls).toContain(sid)
    for (const childSid of childSids) expect(transport.closeCalls).not.toContain(childSid)
    expect(primaryTerm.disposed).toBe(true)

    // The first surviving conn pane is promoted to own the top-level tab.
    const survivor = children.find((c) => c.paneOf === null)!
    const remaining = children.find((c) => c !== survivor)!
    expect(survivor.paneOf).toBeNull()
    expect(remaining.paneOf).toBe(survivor.key)
    expect(controller.activeKey.value).toBe(survivor.key)
    expect(survivor.status).toBe('live')
    expect(survivor.paneRoot).toBeTruthy()
    expect(paneConnTabs(survivor.paneRoot!)).toEqual([survivor, remaining])
    expect(survivor.focusedLeafId).toBe(survivor.key)
  })

  it('splitting a child pane creates another session on the same host', async () => {
    const { controller, transport, tab } = await setup()
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)
    expect(await split(controller, tab, 'down', child.key)).toBe(true)
    await flush()
    const children = controller.tabs.filter((t) => t.paneOf === tab.key)
    expect(children).toHaveLength(2)
    expect(transport.historyCalls).toEqual([
      { id: 'host', expectedRevision: 1 },
      { id: 'host', expectedRevision: 1 }
    ])
    expect(paneConnTabs(tab.paneRoot!)).toEqual([tab, child, children.find((c) => c !== child)!])
  })

  it('primary shell exit promotes a survivor instead of cascading the close', async () => {
    const { controller, transport, tab } = await setup()
    const sid = tab.sessionId!
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)
    const childSid = child.sessionId!

    transport.feed(sid, { bytes: [], exitCode: 0 })
    await flush()

    expect(tab.closed).toBe(true)
    expect(transport.closeCalls).toContain(sid)
    expect(transport.closeCalls).not.toContain(childSid)
    expect(child.closed).toBe(false)
    expect(child.paneOf).toBeNull()
    expect(child.paneRoot).toBeNull()
    expect(child.status).toBe('live')
    expect(controller.activeKey.value).toBe(child.key)
  })

  it('Ctrl+Shift+W on the focused primary pane promotes the survivor', async () => {
    const { controller, transport, tab } = await setup()
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)
    const childSid = child.sessionId!
    tab.focusedLeafId = tab.key

    const closeEvent = new KeyboardEvent('keydown', { code: 'KeyW', ctrlKey: true, shiftKey: true })
    expect(controller.handleKey(closeEvent)).toBe(true)
    await flush()

    expect(tab.closed).toBe(true)
    expect(transport.closeCalls).not.toContain(childSid)
    expect(child.paneOf).toBeNull()
    expect(controller.activeKey.value).toBe(child.key)
  })

  it('removes the mirror leaf when its watched child pane closes', async () => {
    const { controller, tab } = await setup()
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)
    expect(controller.mirrorPane(tab, child.key, 'down')).toBe(true)
    await flush()
    expect(findPaneLeaf(tab.paneRoot!, `mirror:${child.key}`)).toBeTruthy()

    await controller.closeTab(child)
    await flush()
    expect(tab.paneRoot).toBeNull()
  })

  it('child panes keep pane shortcuts out of xterm while the split exists', async () => {
    const { controller, terms, tab } = await setup()
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)
    const childTerm = child.term as unknown as SplitTerm
    const primaryTerm = tab.term as unknown as SplitTerm
    const paneKey = new KeyboardEvent('keydown', { code: 'ArrowRight', ctrlKey: true, altKey: true })
    const plainKey = new KeyboardEvent('keydown', { key: 'a' })

    // false = xterm must not handle it (the global pane handler does).
    expect(childTerm.keyInterceptor!(paneKey)).toBe(false)
    expect(primaryTerm.keyInterceptor!(paneKey)).toBe(false)
    expect(childTerm.keyInterceptor!(plainKey)).toBe(true)
    expect(terms.filter((t) => !t.disposed)).toHaveLength(2)
  })

  it('focuses a clicked pane terminal immediately without waiting for props', async () => {
    const { wrapper, controller, terms, tab } = await setup()
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)
    const childTerm = terms.find((t) => t === (child.term as unknown as SplitTerm))!
    tab.focusedLeafId = tab.key
    await flush()

    const cells = wrapper.findAll('.pane-cell')
    expect(cells).toHaveLength(2)
    const before = childTerm.focusCount
    cells[1].find('.term-host').element.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0 }))
    expect(childTerm.focusCount).toBe(before + 1)
    expect(tab.focusedLeafId).toBe(child.key)
  })

  it('shows the pane menu on a single unsplit pane and splits via it', async () => {
    const { wrapper, controller, tab } = await setup()
    expect(tab.paneRoot).toBeNull()
    const cell = wrapper.find('.pane-cell')
    expect(cell.exists()).toBe(true)
    await cell.trigger('contextmenu')
    await nextTick()
    const menu = document.body.querySelector('.pane-menu')
    expect(menu).toBeTruthy()
    const buttons = [...menu!.querySelectorAll('button')]
    const labels = buttons.map((b) => b.textContent)
    expect(labels).toContain('向右分割')
    expect(labels).toContain('加入唯讀鏡像')
    expect(labels).not.toContain('選擇連線…')
    buttons.find((b) => b.textContent === '向右分割')!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await flush()
    expect(tab.paneRoot?.kind).toBe('split')
    expect(paneConnTabs(tab.paneRoot!)).toHaveLength(2)
    expect(controller.tabs.filter((t) => t.paneOf === tab.key)).toHaveLength(1)
  })

  it('cycles pane focus with keyboard order and closes the focused child pane', async () => {
    const { controller, transport, tab } = await setup()
    await split(controller, tab, 'right')
    const child = paneChild(controller, tab)

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
    const { controller, transport, tab } = await setup()
    const sid = tab.sessionId!
    await split(controller, tab, 'right')
    await split(controller, tab, 'down')
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
