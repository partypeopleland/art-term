import { describe, expect, it, vi } from 'vitest'
import { createDesktopClipboard } from '../src/desktop/clipboard'
import type { TabState } from '../src/shared/state/controller'
import type { TerminalHandle } from '../src/shared/terminal/handle'

function setup() {
  let selection = ''
  let bracketed = false
  const term = { getSelection: () => selection, bracketedPasteMode: () => bracketed } as TerminalHandle
  const tab = { term, status: 'live', closed: false, sessionId: 's1' } as TabState
  const writeText = vi.fn(async (_text: string) => {})
  const readText = vi.fn(async () => '')
  const send = vi.fn()
  const notice = vi.fn()
  const clipboard = createDesktopClipboard({ activeTab: () => tab, writeText, readText, send, notice })
  const key = (code: string, extra: Partial<KeyboardEvent> = {}) => ({
    type: 'keydown', code, key: code.slice(3).toLowerCase(), altKey: true,
    ctrlKey: false, metaKey: false, shiftKey: false,
    preventDefault: vi.fn(), stopPropagation: vi.fn(), ...extra
  }) as unknown as KeyboardEvent
  return { tab, term, clipboard, writeText, readText, send, notice, key,
    select: (text: string) => { selection = text },
    mode: (value: boolean) => { bracketed = value } }
}

describe('Windows terminal clipboard', () => {
  it('copies selected text with Alt+C; empty selection sends nothing, while Ctrl+C always interrupts', async () => {
    const s = setup()
    const empty = s.key('KeyC')
    expect(s.clipboard.handleKey(empty)).toBe(true)
    await Promise.resolve()
    expect(s.writeText).not.toHaveBeenCalled()
    expect(s.send).not.toHaveBeenCalled()
    s.select('中文\nwrapped line')
    expect(s.clipboard.handleKey(s.key('KeyC'))).toBe(true)
    await vi.waitFor(() => expect(s.writeText).toHaveBeenCalledWith('中文\nwrapped line'))
    expect(s.clipboard.handleKey(s.key('KeyC', { altKey: false, ctrlKey: true }))).toBe(true)
    expect(s.send).toHaveBeenCalledWith(s.tab, '\x03')
    s.select('')
    expect(s.clipboard.handleKey(s.key('KeyC', { altKey: false, ctrlKey: true }))).toBe(true)
    expect(s.send).toHaveBeenCalledTimes(2)
    expect(s.clipboard.handleKey(s.key('KeyV', { altKey: false, ctrlKey: true }))).toBe(false)
  })

  it('auto copies once after selection, but ignores empty selection and failed writes', async () => {
    const s = setup()
    await s.clipboard.selectionFinished(s.tab)
    expect(s.writeText).not.toHaveBeenCalled()
    s.select('selected')
    await s.clipboard.selectionFinished(s.tab)
    await s.clipboard.selectionFinished(s.tab)
    expect(s.writeText).toHaveBeenCalledTimes(1)
    expect(s.notice).toHaveBeenCalledWith('已複製')
    s.select('')
    await s.clipboard.selectionFinished(s.tab)
    expect(s.writeText).toHaveBeenCalledTimes(1)
  })

  it('pastes multiline text through quoted insert or bracketed mode, preserving Chinese, blanks, indentation and trailing newline', async () => {
    const s = setup()
    const text = '第一行\n\t第二行\n\n末行\n'
    s.readText.mockResolvedValue(text)
    expect(s.clipboard.handleKey(s.key('KeyV'))).toBe(true)
    await vi.waitFor(() => expect(s.send).toHaveBeenCalledWith(s.tab, text.replace(/[\n\t]/g, character => `\x16${character}`)))
    s.mode(true)
    s.clipboard.handleKey(s.key('KeyV'))
    await vi.waitFor(() => expect(s.send).toHaveBeenCalledWith(s.tab, `\x1b[200~${text}\x1b[201~`))
  })

  it('does not send paste delimiters or executable control characters from clipboard', async () => {
    const s = setup()
    s.mode(true)
    s.readText.mockResolvedValue('hello\x1b[201~\rbad')
    s.clipboard.handleKey(s.key('KeyV'))
    await vi.waitFor(() => expect(s.notice).toHaveBeenCalled())
    expect(s.send).not.toHaveBeenCalled()
  })

  it('sends a single line without an execution key', async () => {
    const s = setup()
    s.readText.mockResolvedValue('echo 中文')
    s.clipboard.handleKey(s.key('KeyV'))
    await vi.waitFor(() => expect(s.send).toHaveBeenCalledWith(s.tab, 'echo 中文'))
  })

  it('quotes both characters of CRLF in the no-bracketed path', async () => {
    const s = setup()
    s.readText.mockResolvedValue('one\r\ntwo\r\n')
    s.clipboard.handleKey(s.key('KeyV'))
    await vi.waitFor(() => expect(s.send).toHaveBeenCalledWith(s.tab, 'one\x16\r\x16\ntwo\x16\r\x16\n'))
  })
})
