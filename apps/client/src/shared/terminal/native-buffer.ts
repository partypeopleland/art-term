import type { IBuffer, IMarker, Terminal } from '@xterm/xterm'

/** UTF-16 [start, end, foreground RGB/default=-1, background RGB/default=-1, flags]. */
export type TextStyle = [number, number, number, number, number]
export interface StyledText { text: string; styles: TextStyle[] }
export const TERMINAL_PALETTE = [
  0x1b1c21, 0xd16969, 0x6a9955, 0xd7ba7d, 0x569cd6, 0xc586c0, 0x4ec9b0, 0xd7d7dc,
  0x5a5b62, 0xe08b8b, 0x8bb573, 0xe2c98f, 0x7bb3e0, 0xd09fd0, 0x6fd4bd, 0xffffff
]
export function paletteColor(index: number): number {
  if (index < 16) return TERMINAL_PALETTE[index] ?? -1
  if (index >= 232) { const v = 8 + (index - 232) * 10; return (v << 16) | (v << 8) | v }
  const n = index - 16, cube = [0, 95, 135, 175, 215, 255]
  return (cube[Math.floor(n / 36)] << 16) | (cube[Math.floor(n / 6) % 6] << 8) | cube[n % 6]
}
export interface NativeTerminalLine { id: number; text: string; styles: TextStyle[]; wrapped?: boolean }
export interface NativeTerminalBuffer { records: NativeTerminalLine[]; supported: boolean; hasOlder?: boolean; hasNewer?: boolean }
export interface NativeBufferWindow { firstId?: number; limit: number }
export const NATIVE_PAGE_LINES = 256
export const NATIVE_WINDOW_LINES = 1024
// Bound the complete local projection. Exceeding this budget uses xterm rather
// than truncating the user's 5000-line buffer or dropping color metadata.
export const NATIVE_BUFFER_BUDGET = 1280 * 1024

export function readStyledLines(buffer: IBuffer, firstId = 1, window?: NativeBufferWindow): NativeTerminalBuffer {
  const records: NativeTerminalLine[] = []
  let size = 0
  if (window && (!Number.isSafeInteger(window.limit) || window.limit < 1
    || (window.firstId !== undefined && !Number.isSafeInteger(window.firstId)))) return { records, supported: false }
  const limit = window ? Math.min(window.limit, NATIVE_WINDOW_LINES) : buffer.length
  const cursor = buffer.baseY + buffer.cursorY
  const first = window?.firstId === undefined ? Math.max(0, Math.min(buffer.length - limit, cursor)) : window.firstId - firstId
  const start = Math.max(0, Math.min(first, Math.max(0, buffer.length - 1)))
  const end = Math.min(buffer.length, start + limit)
  for (let y = start; y < end; y++) {
    const line = buffer.getLine(y)
    if (!line) continue
    let text = ''
    const styles: TextStyle[] = []
    for (let x = 0; x < line.length; x++) {
      const cell = line.getCell(x)
      if (!cell || cell.getWidth() === 0) continue
      const start = text.length
      text += cell.getChars() || ' '
      const index = cell.getFgColor()
      const fg = cell.isFgRGB() ? index : cell.isFgPalette() ? paletteColor(index + (cell.isBold() && index < 8 ? 8 : 0)) : -1
      const bg = cell.isBgRGB() ? cell.getBgColor() : cell.isBgPalette() ? paletteColor(cell.getBgColor()) : -1
      const flags = (cell.isBold() ? 1 : 0) | (cell.isItalic() ? 2 : 0) | (cell.isUnderline() ? 4 : 0)
        | (cell.isStrikethrough() ? 8 : 0) | (cell.isDim() ? 16 : 0) | (cell.isInverse() ? 32 : 0) | (cell.isInvisible() ? 64 : 0)
      if (fg === -1 && bg === -1 && flags === 0) continue
      const last = styles.at(-1)
      if (last && last[1] === start && last[2] === fg && last[3] === bg && last[4] === flags) last[1] = text.length
      else styles.push([start, text.length, fg, bg, flags])
    }
    // Only remove padding cells with the default background. Colored spaces
    // are terminal content and must retain their background spans.
    let end = text.trimEnd().length
    for (const style of styles) if (style[3] !== -1 || (style[4] & (32 | 64))) end = Math.max(end, style[1])
    text = text.slice(0, end)
    const runs = styles.filter(r => r[0] < end).map(r => [r[0], Math.min(r[1], end), r[2], r[3], r[4]] as TextStyle)
    size += text.length * 2 + runs.length * 40 + 32
    if (records.length >= 8192 || text.length > 8192 || runs.length > 8192 || size > NATIVE_BUFFER_BUDGET) return { records: [], supported: false }
    records.push({ id: firstId + y, text, styles: runs, wrapped: line.isWrapped })
  }
  // Blank screen cells below the cursor are not scrollback. Interior blank
  // lines and the cursor's line remain in place.
  while (start + records.length > cursor + 1 && !records.at(-1)!.text) records.pop()
  return { records, supported: true, hasOlder: start > 0, hasNewer: end < buffer.length && end <= cursor }
}

/** A single xterm marker tracks removed head lines without comparing repeated
 * output text. Native anchors use monotonically numbered local buffer lines. */
export function createNativeBufferProjection(term: Terminal): { read(window?: NativeBufferWindow): NativeTerminalBuffer; reset(): void; dispose(): void } {
  let marker: IMarker | undefined
  let previousLine = 0
  let firstId = 1
  let columns: number | undefined
  let reflowed = false
  function reset(): void { marker?.dispose(); marker = undefined; firstId += 10000000; columns = undefined; reflowed = false }
  return {
    read(window) {
      const buffer = term.buffer.active
      // Reflow changes line identities, so retain xterm for this terminal
      // until reset rather than pretending old native anchors still match.
      if (columns === undefined) columns = term.cols
      else if (term.cols !== columns) reflowed = true
      if (reflowed || buffer.type !== 'normal') return { records: [], supported: false }
      if (marker) {
        if (marker.isDisposed) firstId += 10000000
        else firstId += Math.max(0, previousLine - marker.line)
        marker.dispose()
      }
      previousLine = buffer.baseY + buffer.cursorY
      marker = term.registerMarker(0)
      return readStyledLines(buffer, firstId, window)
    },
    reset,
    dispose: () => { marker?.dispose(); marker = undefined }
  }
}
