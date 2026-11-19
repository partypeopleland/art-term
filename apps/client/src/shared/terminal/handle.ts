import { createNativeBufferProjection, type NativeTerminalBuffer, type NativeBufferWindow } from './native-buffer'
// Terminal handle abstraction. The controller only sees this interface, so
// tests can inject a fake without xterm.js or a real DOM renderer.
import type { InjectionKey } from 'vue'
import type { HostTerminalViewport } from './native'
import { Terminal, type IBufferCell, type ITheme } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { attachMobileScroll } from '../../mobile/scroll'
import { attachTerminalLinks, openPreviewWebUrl, type LinkOpener } from './links'

export const SCROLLBACK_LINES = 5000

export interface TerminalHandle {
  setTheme?(theme: ITheme): void
  setHostViewport?(viewport: HostTerminalViewport | null): void
  readonly cols: number
  readonly rows: number
  /** Whether xterm currently has remote bracketed paste mode enabled. */
  bracketedPasteMode?(): boolean
  /** xterm's logical selection omits line wraps but retains real newlines. */
  getSelection?(): string
  hasSelection?(): boolean
  getSelectionPosition?(): { start: { x: number; y: number }; end: { x: number; y: number } } | undefined
  select?(column: number, row: number, length: number): void
  clearSelection?(): void
  readStyledText?(window?: NativeBufferWindow): NativeTerminalBuffer
  /** ANSI snapshot of the current viewport, carried over when the pane remounts. */
  screenSnapshot?(): string
  setNativeReaderActive?(active: boolean): void
  /** Resolves when xterm has parsed the chunk (write-callback backpressure). */
  write(data: Uint8Array, historical?: boolean): Promise<void>
  onData(cb: (text: string, terminalReply?: boolean) => void): void
  /** User input sent outside onData (desktop paste/interrupt shortcuts) must
   * trigger the same host-history follow handling as typed input. */
  noteUserInput?(): void
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

export function createDesktopXtermTerminal(el: HTMLElement, openLink?: LinkOpener): TerminalHandle {
  return createXtermTerminal(el, true, false, openLink)
}

export function createMobileXtermTerminal(el: HTMLElement, openLink?: LinkOpener): TerminalHandle {
  return createXtermTerminal(el, true, true, openLink)
}

function cellSgr(cell: IBufferCell | undefined): string {
  if (!cell) return '0'
  const codes = ['0']
  if (cell.isBold()) codes.push('1')
  if (cell.isDim()) codes.push('2')
  if (cell.isItalic()) codes.push('3')
  if (cell.isUnderline()) codes.push('4')
  if (cell.isBlink()) codes.push('5')
  if (cell.isInverse()) codes.push('7')
  if (cell.isInvisible()) codes.push('8')
  if (cell.isStrikethrough()) codes.push('9')
  if (cell.isOverline()) codes.push('53')
  if (cell.isFgRGB()) {
    const color = cell.getFgColor()
    codes.push(`38;2;${(color >> 16) & 255};${(color >> 8) & 255};${color & 255}`)
  } else if (cell.isFgPalette()) {
    codes.push(`38;5;${cell.getFgColor()}`)
  }
  if (cell.isBgRGB()) {
    const color = cell.getBgColor()
    codes.push(`48;2;${(color >> 16) & 255};${(color >> 8) & 255};${color & 255}`)
  } else if (cell.isBgPalette()) {
    codes.push(`48;5;${cell.getBgColor()}`)
  }
  return codes.join(';')
}

function screenSnapshotAnsi(term: Terminal): string {
  const buffer = term.buffer.active
  const rows = Math.max(1, term.rows)
  const cols = Math.max(1, term.cols)
  const out: string[] = []
  if (buffer.type === 'alternate') out.push('\x1b[?1049h')
  out.push('\x1b[2J\x1b[H\x1b[0m')
  for (let y = 0; y < rows; y++) {
    out.push(`\x1b[${y + 1};1H`)
    let sgr = '0'
    const line = buffer.getLine(buffer.baseY + y)
    for (let x = 0; x < cols; x++) {
      const cell = line?.getCell(x)
      if (cell && cell.getWidth() === 0) continue
      const next = cellSgr(cell)
      if (next !== sgr) {
        out.push(`\x1b[${next}m`)
        sgr = next
      }
      out.push(cell?.getChars() || ' ')
    }
    out.push('\x1b[0m')
    sgr = '0'
  }
  const row = Math.min(Math.max(0, buffer.cursorY) + 1, rows)
  const col = Math.min(Math.max(0, buffer.cursorX) + 1, cols)
  out.push(`\x1b[${row};${col}H\x1b[?25h`)
  return out.join('')
}

export function createXtermTerminal(el: HTMLElement, desktopWheel = false, mobileScroll = false,
  openLink: LinkOpener = openPreviewWebUrl): TerminalHandle {
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
  if (!mobileScroll) el.classList.add('desktop-ime-wrap')
  // xterm positions its composition view at the cursor and forces one line.
  // Keep the original cursor column as the first-line indent while allowing
  // an uncommitted IME preedit to wrap across the terminal width.
  const compositionView = !mobileScroll ? el.querySelector<HTMLElement>('.composition-view') : null
  const compositionTextarea = compositionView ? term.textarea : null
  const compositionScreen = compositionView ? el.querySelector<HTMLElement>('.xterm-screen') : null
  const syncCompositionWrap = () => {
    if (!compositionView || !compositionScreen) return
    compositionView.style.setProperty('--composition-start-x', compositionView.style.left || '0px')
    const screenHeight = compositionScreen.clientHeight
    if (!screenHeight) return
    compositionView.style.maxHeight = `${screenHeight}px`
    compositionView.style.transform = ''
    const top = Number.parseFloat(compositionView.style.top) || 0
    const visibleHeight = compositionView.getBoundingClientRect().height
    compositionView.style.transform = `translateY(-${Math.max(0, top + visibleHeight - screenHeight)}px)`
    compositionView.scrollTop = compositionView.scrollHeight
  }
  compositionTextarea?.addEventListener('compositionupdate', syncCompositionWrap)
  let disposeScroll = mobileScroll ? attachMobileScroll(el, term) : null
  let terminalDisposed = false
  const projection = mobileScroll ? createNativeBufferProjection(term) : null
  const links = attachTerminalLinks(el, term, openLink)
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
  // Public parser observers run before the built-in handler and return false,
  // so only its synchronous protocol-generated onData is marked as a reply.
  let protocolReply = false, replyTurn = 0
  const observeReply = () => {
    protocolReply = true
    const turn = ++replyTurn
    queueMicrotask(() => { if (turn === replyTurn) protocolReply = false })
    return false
  }
  const replyObservers = [
    ...['', '>', '='].map(prefix => term.parser.registerCsiHandler({ prefix, final: 'c' }, observeReply)),
    ...['', '?'].map(prefix => term.parser.registerCsiHandler({ prefix, final: 'n' }, observeReply)),
    ...['', '?'].map(prefix => term.parser.registerCsiHandler({ prefix, intermediates: '$', final: 'p' }, observeReply)),
    term.parser.registerCsiHandler({ final: 't' }, params => [14, 16, 18, 20, 21].includes(Number(params[0])) ? observeReply() : false),
    ...[4, 10, 11, 12].map(id => term.parser.registerOscHandler(id, data => data.split(';').includes('?') ? observeReply() : false)),
    term.parser.registerDcsHandler({ intermediates: '$', final: 'q' }, observeReply),
    term.parser.registerEscHandler({ final: 'Z' }, observeReply)
  ]
  let hostViewport: HostTerminalViewport | null = null
  type SelectionPoint = { x: number; y: number }
  type HostSelection = {
    start: SelectionPoint
    end: SelectionPoint
    offset: number
    instance: string
    cols: number
    rows: number
    text: string
  }
  let hostSelection: HostSelection | null = null
  let applyingHostSelection = false
  // xterm emits deferred selection events after programmatic select/clear;
  // empty captures stay suppressed briefly so they cannot drop the anchor.
  let emptyCaptureSuppressedUntil = 0
  const clearActiveSelection = () => {
    if (!term.hasSelection()) return
    emptyCaptureSuppressedUntil = Date.now() + 500
    applyingHostSelection = true
    term.clearSelection()
    applyingHostSelection = false
  }
  const captureHostSelection = () => {
    hostSelection = null
    if (mobileScroll || !hostViewport || term.buffer.active.type !== 'normal' || !term.hasSelection()) return
    const position = term.getSelectionPosition()
    if (!position) return
    const top = term.buffer.active.baseY
    const bottom = top + term.rows
    if (position.start.y < top || position.end.y < top || position.start.y >= bottom || position.end.y >= bottom) return
    hostSelection = {
      start: { x: position.start.x, y: position.start.y - top },
      end: { x: position.end.x, y: position.end.y - top },
      offset: hostViewport.offset,
      instance: hostViewport.instance,
      cols: term.cols,
      rows: term.rows,
      text: term.getSelection()
    }
  }
  const applyHostSelection = () => {
    if (mobileScroll || !hostViewport || applyingHostSelection) return
    if (!hostSelection && term.buffer.active.type === 'normal' && term.hasSelection()) captureHostSelection()
    const anchor = hostSelection
    if (!anchor) return
    if (anchor.instance !== hostViewport.instance || term.buffer.active.type !== 'normal'
      || term.cols !== anchor.cols || term.rows !== anchor.rows) {
      hostSelection = null
      clearActiveSelection()
      return
    }
    const delta = hostViewport.offset - anchor.offset
    const startY = anchor.start.y + delta
    const endY = anchor.end.y + delta
    const visible = startY >= 0 && endY >= 0 && startY < term.rows && endY < term.rows
    if (!visible) {
      hostSelection = null
      clearActiveSelection()
      return
    }
    const baseY = term.buffer.active.baseY
    const length = (endY - startY) * term.cols + (anchor.end.x - anchor.start.x)
    emptyCaptureSuppressedUntil = Date.now() + 500
    applyingHostSelection = true
    term.select(anchor.start.x, baseY + startY, length)
    applyingHostSelection = false
  }
  if (!mobileScroll) term.onSelectionChange(() => {
    if (applyingHostSelection) return
    if (!term.hasSelection() && Date.now() < emptyCaptureSuppressedUntil) return
    captureHostSelection()
  })
  if (desktopWheel) {
    term.attachCustomWheelEventHandler(event => {
      if (term.buffer.active.type !== 'alternate' || term.modes.mouseTrackingMode !== 'none') return true
      event.preventDefault()
      if (!mobileScroll && hostViewport) {
        const height = el.querySelector('.xterm-rows > div')?.getBoundingClientRect().height || 17
        const lines = event.deltaMode === 1 ? event.deltaY : event.deltaMode === 2 ? Math.sign(event.deltaY) * term.rows : event.deltaY / height
        hostViewport.scroll(lines)
      }
      return false
    })
  }
  const noteUserInput = () => {
    // User input makes the host follow out of history atomically; hide the
    // anchored selection now and let the refreshed view remap or drop it.
    if (hostViewport?.reading) {
      clearActiveSelection()
      hostViewport.noteInput()
    }
  }
  return {
    setTheme: theme => { term.options.theme = theme },
    setHostViewport: viewport => {
      if (mobileScroll) return
      const previous = hostViewport
      if (previous) {
        previous.onView = null
        // Closing a held viewport makes the host follow back to the live
        // screen; selections anchored to the replaced history must not linger.
        if (previous.held || previous.offset !== 0) {
          hostSelection = null
          clearActiveSelection()
        }
      }
      hostViewport = viewport
      if (hostViewport) hostViewport.onView = applyHostSelection
    },
    get cols() {
      return term.cols
    },
    get rows() {
      return term.rows
    },
    bracketedPasteMode: () => term.modes.bracketedPasteMode,
    getSelection: () => hostSelection && term.hasSelection() ? hostSelection.text : term.getSelection(),
    hasSelection: () => term.hasSelection(),
    getSelectionPosition: () => term.getSelectionPosition(),
    select: (column, row, length) => term.select(column, row, length),
    clearSelection: () => term.clearSelection(),
    readStyledText: window => projection?.read(window) ?? { records: [], supported: false },
    screenSnapshot: () => screenSnapshotAnsi(term),
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
        if (historicalParsing) return
        const terminalReply = protocolReply || undefined
        if (!terminalReply) noteUserInput()
        cb(text, terminalReply)
      })
    },
    noteUserInput,
    setKeyInterceptor(fn) {
      term.attachCustomKeyEventHandler(fn)
    },
    fit() {
      try {
        fitAddon.fit()
        disposeScroll?.fit()
        if (compositionView?.classList.contains('active')) syncCompositionWrap()
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
      el.classList.remove('desktop-ime-wrap')
      compositionTextarea?.removeEventListener('compositionupdate', syncCompositionWrap)
      hostViewport = null
      replyObservers.forEach(observer => observer.dispose())
      projection?.dispose()
      links.dispose()
      disposeScroll?.dispose()
      term.dispose()
    }
  }
}
