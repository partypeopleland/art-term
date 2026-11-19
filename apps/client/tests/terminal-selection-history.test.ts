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
  const el = document.createElement('div')
  document.body.append(el)
  const term = createDesktopXtermTerminal(el)
  const rows = term.rows
  const live = Array.from({ length: rows }, (_, y) => `LIVE-${y}`)
  const screen = (offset: number) => Array.from({ length: rows }, (_, y) =>
    offset > 0 && y < offset ? `HIST${offset}-${y}` : live[y - offset])
  const requests: HostHistoryRequest[] = []
  let offset = 0
  const viewport = new HostTerminalViewport(async request => {
    requests.push(request)
    if (request.op === 'close') return { kind: 'ack', instance: 'history-1', op: 'close' }
    if (request.op !== 'scroll') return { kind: 'unsupported' }
    if (request.lines !== 0) {
      offset = Math.max(0, offset - request.lines)
      await writeLines(term, screen(offset))
    }
    return { kind: 'view', instance: 'history-1', op: 'scroll', held: offset > 0, offset, available: true }
  })
  opened.push({ term, viewport })
  return { term, viewport, requests, live, rows }
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
})
