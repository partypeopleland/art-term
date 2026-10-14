import { createNativeBufferProjection, type NativeTerminalBuffer } from './native-buffer'
// Terminal handle abstraction. The controller only sees this interface, so
// tests can inject a fake without xterm.js or a real DOM renderer.
import type { InjectionKey } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { attachMobileScroll } from '../mobile/scroll'
import { attachTerminalLinks, type ClipboardWriter } from './links'

export const SCROLLBACK_LINES = 5000

export interface TerminalHandle {
  readonly cols: number
  readonly rows: number
  /** Whether xterm currently has remote bracketed paste mode enabled. */
  bracketedPasteMode?(): boolean
  readStyledText?(): NativeTerminalBuffer
  setNativeReaderActive?(active: boolean): void
  /** Resolves when xterm has parsed the chunk (write-callback backpressure). */
  write(data: Uint8Array, historical?: boolean): Promise<void>
  onData(cb: (text: string) => void): void
  /** Return false from fn to keep the event out of xterm entirely. */
  setKeyInterceptor(fn: (ev: KeyboardEvent) => boolean): void
  fit(): void
  focus(): void
  suppressSoftKeyboard?(): void
  reset(): void
  dispose(): void
}

export type TerminalFactory = (el: HTMLElement) => TerminalHandle

export const TerminalFactoryKey: InjectionKey<TerminalFactory> = Symbol('TerminalFactory')

export function createDesktopXtermTerminal(el: HTMLElement, writeClipboard?: ClipboardWriter): TerminalHandle {
  return createXtermTerminal(el, true, false, writeClipboard)
}

export function createMobileXtermTerminal(el: HTMLElement, writeClipboard?: ClipboardWriter): TerminalHandle {
  return createXtermTerminal(el, true, true, writeClipboard)
}

export function createXtermTerminal(el: HTMLElement, desktopWheel = false, mobileScroll = false,
  writeClipboard: ClipboardWriter = text => navigator.clipboard.writeText(text)): TerminalHandle {
  const term = new Terminal({
    scrollback: SCROLLBACK_LINES,
    cursorBlink: true,
    fontFamily: 'Consolas, "Cascadia Mono", "Microsoft JhengHei", monospace',
    fontSize: 14,
    lineHeight: 1.2,
    smoothScrollDuration: mobileScroll ? 100 : 0,
    theme: {
      background: '#16171b',
      foreground: '#d7d7dc',
      cursor: '#d7d7dc',
      cursorAccent: '#16171b',
      selectionBackground: '#3a4a63',
      black: '#1b1c21',
      red: '#d16969',
      green: '#6a9955',
      yellow: '#d7ba7d',
      blue: '#569cd6',
      magenta: '#c586c0',
      cyan: '#4ec9b0',
      white: '#d7d7dc',
      brightBlack: '#5a5b62',
      brightRed: '#e08b8b',
      brightGreen: '#8bb573',
      brightYellow: '#e2c98f',
      brightBlue: '#7bb3e0',
      brightMagenta: '#d09fd0',
      brightCyan: '#6fd4bd',
      brightWhite: '#ffffff'
    }
  })
  const fitAddon = new FitAddon()
  term.loadAddon(fitAddon)
  term.open(el)
  let disposeScroll = mobileScroll ? attachMobileScroll(el, term) : null
  let terminalDisposed = false
  const projection = mobileScroll ? createNativeBufferProjection(term) : null
  const links = attachTerminalLinks(el, term, writeClipboard)
  let historicalParsing = false
  // A query may start in replay and finish in a live frame. Keep its replay
  // origin until the escape sequence ends, rather than replying to its suffix.
  let parserState: 'ground' | 'escape' | 'csi' | 'string' | 'stringEscape' = 'ground'
  let historicalSequence = false
  function replySegments(data: Uint8Array, historical: boolean): { bytes: Uint8Array; suppress: boolean }[] {
    const segments: { bytes: Uint8Array; suppress: boolean }[] = []
    let start = 0
    let previous = historical || historicalSequence
    for (let i = 0; i < data.length; i++) {
      const suppress = historical || historicalSequence
      if (suppress !== previous) {
        segments.push({ bytes: data.subarray(start, i), suppress: previous })
        start = i
        previous = suppress
      }
      const byte = data[i]
      if (byte === 0x18 || byte === 0x1a) {
        parserState = 'ground'
      } else if (parserState === 'ground') {
        if (byte === 0x1b) parserState = 'escape'
      } else if (parserState === 'escape') {
        if (byte === 0x5b) parserState = 'csi'
        else if ([0x5d, 0x50, 0x58, 0x5e, 0x5f].includes(byte)) parserState = 'string'
        else if (byte >= 0x30 && byte <= 0x7e) parserState = 'ground'
      } else if (parserState === 'csi') {
        if (byte === 0x1b) parserState = 'escape'
        else if (byte >= 0x40 && byte <= 0x7e) parserState = 'ground'
      } else if (parserState === 'string') {
        if (byte === 0x07) parserState = 'ground'
        else if (byte === 0x1b) parserState = 'stringEscape'
      } else if (parserState === 'stringEscape') {
        parserState = byte === 0x5c ? 'ground' : 'string'
      }
      historicalSequence = parserState !== 'ground' && (historicalSequence || historical)
    }
    if (start < data.length) segments.push({ bytes: data.subarray(start), suppress: previous })
    return segments
  }
  if (desktopWheel) {
    term.attachCustomWheelEventHandler(event => {
      if (term.buffer.active.type !== 'alternate' || term.modes.mouseTrackingMode !== 'none') return true
      // Keep native mouse reporting for mouse-enabled tmux. Otherwise
      // prevent xterm's wheel-to-arrow fallback, including the
      // startup window before tmux enables mouse. Keyboard arrows stay intact.
      event.preventDefault()
      return false
    })
  }
  return {
    get cols() {
      return term.cols
    },
    get rows() {
      return term.rows
    },
    bracketedPasteMode: () => term.modes.bracketedPasteMode,
    readStyledText: () => projection?.read() ?? { records: [], supported: false },
    setNativeReaderActive(active) {
      if (!mobileScroll || terminalDisposed) return
      if (active) { disposeScroll?.dispose(); disposeScroll = null; el.classList.add('mobile-terminal') }
      else if (!disposeScroll) disposeScroll = attachMobileScroll(el, term)
    },
    async write(data, historical = false) {
      for (const segment of replySegments(data, historical)) {
        historicalParsing = segment.suppress
        try {
          await new Promise<void>((resolve) => term.write(segment.bytes, resolve))
        } finally {
          historicalParsing = false
        }
      }
    },
    onData(cb) {
      term.onData(text => {
        if (!historicalParsing) cb(text)
      })
    },
    setKeyInterceptor(fn) {
      term.attachCustomKeyEventHandler(fn)
    },
    fit() {
      try {
        fitAddon.fit()
        disposeScroll?.fit()
      } catch {
        // Element hidden or zero-sized; keep last stable size.
      }
    },
    focus() {
      const textarea = term.textarea
      if (textarea?.inputMode === 'none') {
        textarea.inputMode = 'text'
        textarea.blur()
      }
      term.focus()
    },
    suppressSoftKeyboard() {
      // Keep hardware input available, but prevent WebView's implicit IME
      // request when touching a toolbar with a focused terminal textarea.
      if (term.textarea) term.textarea.inputMode = 'none'
    },
    reset() {
      projection?.reset()
      parserState = 'ground'
      historicalSequence = false
      disposeScroll?.reset()
      links.reset()
      term.reset()
    },
    dispose() {
      terminalDisposed = true
      projection?.dispose()
      links.dispose()
      disposeScroll?.dispose()
      term.dispose()
    }
  }
}
