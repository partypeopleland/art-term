import type { IBufferRange, ILink, Terminal } from '@xterm/xterm'

export type ClipboardWriter = (text: string) => Promise<void>

function contains(range: IBufferRange, x: number, y: number): boolean {
  return y >= range.start.y && y <= range.end.y &&
    (y !== range.start.y || x >= range.start.x) && (y !== range.end.y || x <= range.end.x)
}

/** Use buffer cells rather than DOM text: wide characters and wrapped URLs keep their coordinates. */
function linksForLine(term: Terminal, y: number, activate: ILink['activate']): ILink[] {
  const buffer = term.buffer.active
  let start = y - 1, end = start
  while (start > 0 && buffer.getLine(start)?.isWrapped && y - start < 64) start--
  while (end + 1 < buffer.length && buffer.getLine(end + 1)?.isWrapped && end - start < 64) end++
  let text = ''
  const cells: { x: number; y: number; width: number }[] = []
  for (let row = start; row <= end && text.length < 32768; row++) {
    const line = buffer.getLine(row)
    if (!line) break
    for (let col = 0; col < term.cols; col++) {
      const cell = line.getCell(col), width = cell?.getWidth() ?? 1
      if (!width) continue
      const chars = cell?.getChars() || ' '
      text += chars
      for (let i = 0; i < chars.length; i++) cells.push({ x: col + 1, y: row + 1, width })
    }
  }
  const links: ILink[] = []
  for (const match of text.matchAll(/https?:\/\/[^\s<>"'`]+/gi)) {
    let url = match[0].replace(/[.,;:!?，。；！？、”》]+$/, '')
    // Retain balanced parentheses in URLs, but drop surrounding Markdown punctuation.
    while (/[)\]}]$/.test(url)) {
      const close = url.at(-1)!, open = { ')': '(', ']': '[', '}': '{' }[close]!
      if (url.split(close).length <= url.split(open).length) break
      url = url.slice(0, -1)
    }
    try { if (!new URL(url).hostname) continue } catch { continue }
    const first = cells[match.index!], last = cells[match.index! + url.length - 1]
    if (!first || !last) continue
    const range = { start: { x: first.x, y: first.y }, end: { x: last.x + last.width - 1, y: last.y } }
    if (y >= range.start.y && y <= range.end.y) links.push({ text: url, range, activate })
  }
  return links
}

/** Link taps are local actions, even when a remote TUI has mouse reporting enabled. */
export function attachTerminalLinks(host: HTMLElement, term: Terminal, writeClipboard: ClipboardWriter) {
  const screen = host.querySelector<HTMLElement>('.xterm-screen')!
  const notice = document.createElement('div')
  notice.className = 'terminal-link-notice'
  notice.setAttribute('role', 'status')
  notice.setAttribute('aria-live', 'polite')
  notice.hidden = true
  host.append(notice)
  let disposed = false, timer: ReturnType<typeof setTimeout> | undefined, generation = 0
  let hovered: { text: string; range: IBufferRange } | undefined
  let press: { text: string; x: number; y: number; touch: boolean; moved: boolean } | undefined
  const stop = (event: Event) => { event.preventDefault(); event.stopImmediatePropagation() }
  const activate: ILink['activate'] = (event, url) => {
    stop(event)
    const current = ++generation
    void Promise.resolve().then(() => writeClipboard(url))
      .then(() => show('已複製連結', current), () => show('複製失敗，請再點一次連結', current))
  }
  function show(message: string, current: number): void {
    if (disposed || current !== generation) return
    clearTimeout(timer)
    notice.textContent = message
    notice.hidden = false
    timer = setTimeout(() => { notice.hidden = true }, 2500)
  }
  term.options.linkHandler = {
    activate,
    hover: (_event, text, range) => { hovered = { text, range } },
    leave: () => { hovered = undefined }
  }
  const provider = term.registerLinkProvider({ provideLinks: (y, callback) => callback(linksForLine(term, y, activate)) })
  function at(clientX: number, clientY: number): string | undefined {
    const rect = screen.getBoundingClientRect()
    if (!rect.width || !rect.height || clientX < rect.left || clientX >= rect.right || clientY < rect.top || clientY >= rect.bottom) return
    const x = Math.floor((clientX - rect.left) * term.cols / rect.width) + 1
    const y = Math.floor((clientY - rect.top) * term.rows / rect.height) + term.buffer.active.viewportY + 1
    return (hovered && contains(hovered.range, x, y) ? hovered.text : undefined) ??
      linksForLine(term, y, activate).find(link => contains(link.range, x, y))?.text
  }
  function mouseDown(event: MouseEvent): void {
    if (event.button !== 0) return
    const text = at(event.clientX, event.clientY)
    press = text ? { text, x: event.clientX, y: event.clientY, touch: false, moved: false } : undefined
    if (press) stop(event)
  }
  function mouseUp(event: MouseEvent): void {
    if (!press || press.touch || event.button !== 0) return
    const pending = press
    press = undefined
    stop(event)
    if (Math.hypot(event.clientX - pending.x, event.clientY - pending.y) <= 5 && at(event.clientX, event.clientY) === pending.text) activate(event, pending.text)
  }
  function touchStart(event: TouchEvent): void {
    press = undefined
    if (event.touches.length !== 1) return
    const finger = event.touches[0], text = at(finger.clientX, finger.clientY)
    if (text) press = { text, x: finger.clientX, y: finger.clientY, touch: true, moved: false }
  }
  function touchMove(event: TouchEvent): void {
    if (!press?.touch) return
    const finger = event.touches[0]
    if (event.touches.length !== 1 || !finger || Math.hypot(finger.clientX - press.x, finger.clientY - press.y) >= 5) press.moved = true
  }
  function touchEnd(event: TouchEvent): void {
    if (!press?.touch) return
    const pending = press, finger = event.changedTouches[0]
    press = undefined
    if (pending.moved || event.touches.length || !finger || Math.hypot(finger.clientX - pending.x, finger.clientY - pending.y) >= 5 || at(finger.clientX, finger.clientY) !== pending.text) return
    // Prevent the compatibility mouse events from reaching Codex's URL opener.
    activate(event as unknown as MouseEvent, pending.text)
  }
  const cancel = () => { press = undefined; hovered = undefined }
  const click = (event: MouseEvent) => { if (at(event.clientX, event.clientY)) stop(event) }
  host.addEventListener('mousedown', mouseDown, true)
  host.addEventListener('mouseup', mouseUp, true)
  host.addEventListener('click', click, true)
  host.addEventListener('touchstart', touchStart, { capture: true, passive: true })
  host.addEventListener('touchmove', touchMove, { capture: true, passive: true })
  host.addEventListener('touchend', touchEnd, { capture: true, passive: false })
  host.addEventListener('touchcancel', cancel, true)
  window.addEventListener('blur', cancel)
  const subscriptions = [term.onWriteParsed(() => { hovered = undefined }), term.onScroll(cancel), term.onResize(cancel)]
  return {
    reset: cancel,
    dispose() {
      disposed = true
      clearTimeout(timer)
      provider.dispose()
      subscriptions.forEach(subscription => subscription.dispose())
      host.removeEventListener('mousedown', mouseDown, true)
      host.removeEventListener('mouseup', mouseUp, true)
      host.removeEventListener('click', click, true)
      host.removeEventListener('touchstart', touchStart, true)
      host.removeEventListener('touchmove', touchMove, true)
      host.removeEventListener('touchend', touchEnd, true)
      host.removeEventListener('touchcancel', cancel, true)
      window.removeEventListener('blur', cancel)
      notice.remove()
    }
  }
}
