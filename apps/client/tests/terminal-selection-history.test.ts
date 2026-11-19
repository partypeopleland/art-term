import { afterEach, describe, expect, it, vi } from 'vitest'
import { createDesktopXtermTerminal, type TerminalHandle } from '../src/shared/terminal/handle'
import { HostTerminalViewport, type HostHistoryRequest } from '../src/shared/terminal/native'

vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
window.matchMedia = (() => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia

const opened: { term: TerminalHandle; viewport: HostTerminalViewport }[] = []
afterEach(async () => {
  for (const item of opened.splice(0)) {
    await item.viewport.close().catch(() => undefined)
    item.term.dispose()
  }
  document.body.replaceChildren()
})

const bytes = (text: string) => new TextEncoder().encode(text)
const selectionText = (term: TerminalHandle) => term.getSelection!()
const selectionActive = (term: TerminalHandle) => term.hasSelection!()
const selectionPosition = (term: TerminalHandle) => term.getSelectionPosition!()!
const selectRange = (term: TerminalHandle, column: number, row: number, length: number) => term.select!(column, row, length)

async function writeLines(term: TerminalHandle, lines: string[]): Promise<void> {
  const body = lines.map((text, y) => `\x1b[${y + 1};1H${text}\x1b[K`).join('')
  await term.write(bytes(`\x1b[2J\x1b[H${body}`))
}

function setup() {
  const host = document.createElement('div')
  document.body.append(host)
  const term = createDesktopXtermTerminal(host)
  const rows = term.rows
  const live = Array.from({ length: rows }, (_, y) => `LIVE-${y}`)
  const screen = (offset: number) => Array.from({ length: rows }, (_, y) =>
    offset > 0 && y < offset ? `HIST${offset}-${y}` : live[y - offset])
  const requests: HostHistoryRequest[] = []
  let offset = 0
  let instance = 'history-1'
  const viewport = new HostTerminalViewport(async request => {
    requests.push(request)
    if (request.op === 'close') return { kind: 'ack', instance, op: 'close' }
    if (request.op === 'follow') {
      offset = 0
      await writeLines(term, live)
      return { kind: 'view', instance, op: 'follow', held: false, offset, available: true }
    }
    if (request.op !== 'scroll') return { kind: 'unsupported' }
    if (request.lines !== 0) {
      offset = Math.max(0, offset - request.lines)
      await writeLines(term, screen(offset))
    }
    return { kind: 'view', instance, op: 'scroll', held: offset > 0, offset, available: true }
  })
  opened.push({ term, viewport })
  return { host, term, viewport, requests, live, rows, setInstance: (value: string) => { instance = value } }
}

describe('desktop host-history selection', () => {
  it('keeps the selection on the same text while the range stays visible', async () => {
    const { term, viewport, live, rows } = setup()
    await writeLines(term, live)
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    selectRange(term, 0, 5, live[5].length)
    expect(selectionText(term)).toBe(live[5])

    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    expect(selectionText(term)).toBe(live[5])
    expect(selectionPosition(term).start.y).toBe(8)

    viewport.scroll(3)
    await vi.waitFor(() => expect(viewport.offset).toBe(0))
    expect(selectionText(term)).toBe(live[5])
    expect(selectionPosition(term).start.y).toBe(5)
    expect(rows).toBeGreaterThan(8)
  })

  it('clears the selection when the selected range leaves the visible viewport', async () => {
    const { term, viewport, live, rows } = setup()
    await writeLines(term, live)
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    const bottom = rows - 1
    selectRange(term, 0, bottom, live[bottom].length)
    expect(selectionText(term)).toBe(live[bottom])

    viewport.scroll(-5)
    await vi.waitFor(() => expect(viewport.offset).toBe(5))
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')

    viewport.scroll(5)
    await vi.waitFor(() => expect(viewport.offset).toBe(0))
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')
  })

  it('clears a history-only selection when the pane returns to follow', async () => {
    const { term, viewport, live } = setup()
    await writeLines(term, live)
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    selectRange(term, 0, 0, 'HIST3-0'.length)
    expect(selectionText(term)).toBe('HIST3-0')

    viewport.scroll(3)
    await vi.waitFor(() => expect(viewport.offset).toBe(0))
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')

    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')
  })

  it('recomputes a live selection and clears history text when typing follows to live', async () => {
    const { host, term, viewport, live } = setup()
    await writeLines(term, live)
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    selectRange(term, 0, 5, live[5].length)
    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    expect(selectionPosition(term).start.y).toBe(8)

    const input: string[] = []
    term.onData(text => input.push(text))
    host.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', keyCode: 13, bubbles: true, cancelable: true }))
    await vi.waitFor(() => expect(input).toEqual(['\r']))
    await vi.waitFor(() => expect(viewport.offset).toBe(0))
    expect(viewport.held).toBe(false)
    expect(selectionText(term)).toBe(live[5])
    expect(selectionPosition(term).start.y).toBe(5)
  })

  it('clears a history-only selection when typing follows to live', async () => {
    const { host, term, viewport } = setup()
    await writeLines(term, Array.from({ length: term.rows }, (_, y) => `LIVE-${y}`))
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    selectRange(term, 0, 0, 'HIST3-0'.length)
    expect(selectionText(term)).toBe('HIST3-0')

    term.onData(() => {})
    host.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', keyCode: 13, bubbles: true, cancelable: true }))
    await vi.waitFor(() => expect(viewport.offset).toBe(0))
    expect(viewport.held).toBe(false)
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')
  })

  it('clears the selection when the viewport detaches while reading history', async () => {
    const { term, viewport } = setup()
    await writeLines(term, Array.from({ length: term.rows }, (_, y) => `LIVE-${y}`))
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    selectRange(term, 0, 0, 'HIST3-0'.length)
    expect(selectionText(term)).toBe('HIST3-0')

    term.setHostViewport?.(null)
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')
  })

  it('keeps a live selection when the viewport detaches at the bottom', async () => {
    const { term, viewport, live } = setup()
    await writeLines(term, live)
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    selectRange(term, 0, 5, live[5].length)
    expect(selectionText(term)).toBe(live[5])

    term.setHostViewport?.(null)
    expect(selectionActive(term)).toBe(true)
    expect(selectionText(term)).toBe(live[5])
  })

  it('clears the selection and its text when the history instance changes', async () => {
    const { term, viewport, live, setInstance } = setup()
    await writeLines(term, live)
    term.setHostViewport?.(viewport)
    viewport.start()
    await vi.waitFor(() => expect(viewport.instance).toBe('history-1'))

    selectRange(term, 0, 5, live[5].length)
    viewport.scroll(-3)
    await vi.waitFor(() => expect(viewport.offset).toBe(3))
    expect(selectionText(term)).toBe(live[5])

    setInstance('history-2')
    viewport.scroll(-1)
    await vi.waitFor(() => expect(viewport.instance).toBe('history-2'))
    expect(selectionActive(term)).toBe(false)
    expect(selectionText(term)).toBe('')
  })
})
