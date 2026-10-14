import { expect, it } from 'vitest'
import { Terminal } from '@xterm/xterm'
import { createNativeBufferProjection, readStyledLines } from '../src/terminal/native-buffer'
const write = (term: Terminal, value: string) => new Promise<void>(resolve => term.write(value, resolve))

it('projects parsed Unicode, palette/RGB and colored padding without modifying terminal input', async () => {
  const term = new Terminal({ cols: 80, rows: 2 })
  const input: string[] = []
  term.onData(value => input.push(value))
  await write(term, '\x1b[31m紅😀\x1b[1m亮\x1b[0m plain \x1b[38;2;18;52;86mRGB\x1b[0m')
  const result = readStyledLines(term.buffer.active)
  expect(result.supported).toBe(true)
  expect(result.records[0].text).toBe('紅😀亮 plain RGB')
  expect(result.records[0].styles).toEqual([[0, 3, 0xd16969, -1, 0], [3, 4, 0xe08b8b, -1, 1], [11, 14, 0x123456, -1, 0]])
  await write(term, '\r\n\x1b[44m  \x1b[0m')
  expect(readStyledLines(term.buffer.active).records.at(-1)?.styles[0][3]).toBe(0x569cd6)
  expect(input).toEqual([])
  term.dispose()
})

it('keeps the full 5000-line local scrollback rather than a 512-line tail', async () => {
  const term = new Terminal({ cols: 80, rows: 2, scrollback: 5000 })
  await write(term, Array.from({ length: 5001 }, (_, n) => `line-${n}`).join('\r\n'))
  const result = readStyledLines(term.buffer.active)
  expect(result.supported).toBe(true)
  expect(result.records).toHaveLength(5001)
  expect(result.records[0].text).toBe('line-0')
  term.dispose()
})

it('keeps native line IDs stable through append and head trim, even for duplicate lines', async () => {
  const term = new Terminal({ cols: 80, rows: 2, scrollback: 3 })
  const projection = createNativeBufferProjection(term)
  await write(term, 'same\r\nsame\r\nsame')
  const before = projection.read()
  const id = before.records[2].id
  await write(term, '\r\nsame\r\nsame\r\nsame')
  const after = projection.read()
  expect(after.records.find(r => r.id === id)?.text).toBe('same')
  expect(after.records[1].id).toBe(id)
  projection.dispose(); term.dispose()
})

it('falls back for alternate screens, reflow and oversized content, retaining the xterm buffer', async () => {
  const term = new Terminal({ cols: 80, rows: 2 })
  const projection = createNativeBufferProjection(term)
  await write(term, 'normal')
  projection.read()
  await write(term, '\x1b[?1049h')
  expect(projection.read().supported).toBe(false)
  await write(term, '\x1b[?1049l')
  expect(projection.read().supported).toBe(true)
  term.resize(60, 2)
  expect(projection.read().supported).toBe(false)
  expect(term.buffer.normal.getLine(0)?.translateToString(true)).toBe('normal')
  projection.reset()
  expect(projection.read().supported).toBe(true)
  const wide = new Terminal({ cols: 9000, rows: 2 })
  await write(wide, 'x'.repeat(8500))
  expect(readStyledLines(wide.buffer.active).supported).toBe(false)
  expect(wide.buffer.active.getLine(0)?.translateToString(true)).toHaveLength(8500)
  projection.dispose(); term.dispose(); wide.dispose()
})

it('pins columns after the initial mobile fit and only falls back on later reflow', async () => {
  const term = new Terminal({ cols: 80, rows: 24 })
  const projection = createNativeBufferProjection(term)
  term.resize(44, 18)
  await write(term, 'mobile')
  expect(projection.read().supported).toBe(true)
  term.resize(44, 10)
  expect(projection.read().supported).toBe(true)
  term.resize(55, 10)
  expect(projection.read().supported).toBe(false)
  projection.dispose(); term.dispose()
})

it('marks xterm soft wraps so a visible URL is not split into separate native links', async () => {
  const term = new Terminal({ cols: 20, rows: 3 })
  await write(term, 'https://example.com/long-url-path')
  const result = readStyledLines(term.buffer.active)
  expect(result.records[1].wrapped).toBe(true)
  const text = result.records.map((r, n) => (n && !r.wrapped ? '\n' : '') + r.text).join('')
  expect(text).toBe('https://example.com/long-url-path')
  term.dispose()
})
