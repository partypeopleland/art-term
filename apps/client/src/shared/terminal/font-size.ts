import { ref } from 'vue'

export const TERMINAL_FONT_SIZE_DEFAULT = 14
export const TERMINAL_FONT_SIZE_MIN = 10
export const TERMINAL_FONT_SIZE_MAX = 32
export const TERMINAL_FONT_SIZE_STEP = 1

/** Desktop terminal font size shared by every terminal pane in the window. */
export const terminalFontSize = ref(TERMINAL_FONT_SIZE_DEFAULT)

export function clampTerminalFontSize(size: number): number {
  if (!Number.isFinite(size)) return TERMINAL_FONT_SIZE_DEFAULT
  return Math.min(TERMINAL_FONT_SIZE_MAX, Math.max(TERMINAL_FONT_SIZE_MIN, Math.round(size)))
}

export function zoomTerminalFontSize(direction: 1 | -1): number {
  const next = clampTerminalFontSize(terminalFontSize.value + direction * TERMINAL_FONT_SIZE_STEP)
  terminalFontSize.value = next
  return next
}

/** Ctrl+= / Ctrl+Shift+= (`+`) and Ctrl+Minus / Ctrl+NumpadSubtract keys. */
export function terminalFontZoomKeyDirection(event: KeyboardEvent): 1 | -1 | null {
  if (event.type !== 'keydown' || !event.ctrlKey || event.altKey || event.metaKey) return null
  if (event.code === 'Equal' || event.code === 'NumpadAdd') return 1
  if (event.code === 'Minus' || event.code === 'NumpadSubtract') return -1
  return null
}

/** Ctrl+wheel up zooms in, Ctrl+wheel down zooms out; plain wheel is untouched. */
export function terminalFontZoomWheelDirection(event: WheelEvent): 1 | -1 | null {
  if (event.type !== 'wheel' || !event.ctrlKey || event.altKey || event.metaKey || event.deltaY === 0) return null
  return event.deltaY < 0 ? 1 : -1
}
