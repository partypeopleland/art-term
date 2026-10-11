import type { Terminal } from '@xterm/xterm'
import './scroll.css'

/** Read tmux's rendered copy-mode position instead of inventing a remote extent. */
export function tmuxHistoryPosition(line: string): { offset: number; total: number } | null {
  const match = /\[(\d+)\/(\d+)\]\s*$/.exec(line)
  if (!match) return null
  const offset = Number(match[1]), total = Number(match[2])
  return total > 0 && offset <= total ? { offset, total } : null
}

export function attachMobileScroll(host: HTMLElement, term: Terminal): { reset(): void; dispose(): void; fit(): void } {
  const screen = host.querySelector<HTMLElement>('.xterm-screen')!
  host.classList.add('mobile-terminal')
  const rail = document.createElement('div'); rail.className = 'mobile-scrollbar'
  const track = document.createElement('div'); track.className = 'mobile-scroll-track'; track.tabIndex = 0
  track.setAttribute('aria-label', '上下拖曳捲動歷史')
  const thumb = document.createElement('div'); thumb.className = 'mobile-scroll-thumb'
  track.append(thumb); rail.append(track); host.append(rail)
  let disposed = false, frame = 0, pendingWheel = 0, lastWheel = 0
  let remoteHistory: { offset: number; total: number } | null = null
  let remoteTarget: number | null = null
  let targetDeadline = 0
  let touch: { id: number; x: number; y: number; startY: number; active: boolean; time: number; speed: number } | null = null
  let drag: { id: number; y: number } | null = null
  // Keep the gesture hit location: full-screen apps ignore wheel events outside their transcript.
  let wheelOrigin: { x: number; y: number } | null = null
  let inertia = 0, lastFrame = 0
  let previousPosition: number | null = null, scrollDelta = 0
  let previousRows: HTMLElement | null = null, oldRows: HTMLElement | null = null
  let rowAnimation: Animation | null = null
  const remote = () => term.modes.mouseTrackingMode !== 'none'
  const rowHeight = () => Math.max(8, screen.getBoundingClientRect().height / Math.max(1, term.rows))
  function localScroll(pixels: number): void { term.scrollLines(pixels / rowHeight()) }
  function wheel(direction: number): void {
    const r = screen.getBoundingClientRect()
    if (r.width <= 0 || r.height <= 0) return
    const origin = wheelOrigin ?? { x: .5, y: .5 }
    screen.dispatchEvent(new WheelEvent('wheel', { deltaY: direction, deltaMode: 1, bubbles: true, cancelable: true,
      clientX: r.left + Math.max(1, Math.min(r.width - 1, origin.x * r.width)),
      clientY: r.top + Math.max(1, Math.min(r.height - 1, origin.y * r.height)) }))
  }
  function animate(time: number): void {
    frame = 0
    if (disposed) return
    const elapsed = Math.min(32, time - (lastFrame || time)); lastFrame = time
    if (inertia && !remote() && term.buffer.active.type === 'normal') {
      localScroll(inertia * elapsed); inertia *= Math.exp(-elapsed / 180)
      if (Math.abs(inertia) < .03) inertia = 0
    }
    if (remoteTarget !== null && remoteHistory) {
      if (time > targetDeadline) remoteTarget = null
      if (remoteTarget !== null) {
        const diff = remoteHistory.offset - remoteTarget
        if (Math.abs(diff) < 3) remoteTarget = null
        else pendingWheel = Math.sign(diff)
      }
    }
    if (pendingWheel && time - lastWheel >= 48) {
      const direction = Math.sign(pendingWheel); pendingWheel -= direction
      wheel(direction); lastWheel = time
    }
    if (inertia || pendingWheel || remoteTarget !== null) schedule()
  }
  function schedule(): void { if (!frame && !disposed) frame = requestAnimationFrame(animate) }
  function scrollLines(lines: number): void {
    if (remote()) { pendingWheel = Math.max(-8, Math.min(8, pendingWheel + lines)); schedule() }
    else if (term.buffer.active.type === 'normal') term.scrollLines(lines)
  }
  function cancel(): void {
    touch = null; drag = null; wheelOrigin = null; remoteTarget = null; pendingWheel = 0; inertia = 0; lastFrame = 0
    if (frame) cancelAnimationFrame(frame)
    frame = 0
    rowAnimation?.cancel(); rowAnimation = null; oldRows?.remove(); oldRows = null
  }
  function update(): void {
    if (disposed) return
    const buffer = term.buffer.active, isRemote = remote() || buffer.type === 'alternate'
    rail.dataset.mode = isRemote ? 'remote' : 'local'
    if (remote() && buffer.type === 'alternate') {
      const position = tmuxHistoryPosition(buffer.getLine(buffer.viewportY)?.translateToString(true) ?? '')
      if (position) remoteHistory = position
      else if (remoteHistory) remoteHistory = { ...remoteHistory, offset: 0 }
    }
    if (!isRemote) remoteHistory = null
    const total = isRemote ? remoteHistory?.total : buffer.baseY
    const position = isRemote ? (remoteHistory ? remoteHistory.total - remoteHistory.offset : null) : buffer.viewportY
    if (position !== null && previousPosition !== null) scrollDelta += previousPosition - position
    previousPosition = position
    thumb.hidden = total === undefined || position === null
    if (thumb.hidden) {
      track.setAttribute('role', 'group'); ['aria-valuemin', 'aria-valuemax', 'aria-valuenow'].forEach(a => track.removeAttribute(a)); return
    }
    track.setAttribute('role', 'scrollbar'); track.setAttribute('aria-orientation', 'vertical')
    track.setAttribute('aria-valuemin', '0'); track.setAttribute('aria-valuemax', String(total)); track.setAttribute('aria-valuenow', String(position))
    const height = track.clientHeight
    const size = Math.min(height, Math.max(24, height * term.rows / Math.max(term.rows, (total ?? 0) + term.rows)))
    thumb.style.height = `${size}px`
    thumb.style.top = `${total ? (height - size) * position! / total : 0}px`
  }
  function fit(): void {
    rail.style.height = `${Math.min(screen.offsetHeight, Math.max(0, host.clientHeight - 8))}px`
    rail.style.bottom = 'auto'; update()
  }
  function render(): void {
    const rows = screen.querySelector<HTMLElement>('.xterm-rows')
    if (!rows) return
    const snapshot = rows.cloneNode(true) as HTMLElement
    if (previousRows && scrollDelta && Math.abs(scrollDelta) <= term.rows && typeof rows.animate === 'function') {
      rowAnimation?.cancel(); oldRows?.remove()
      const shift = scrollDelta * rowHeight()
      oldRows = previousRows; oldRows.setAttribute('aria-hidden', 'true')
      Object.assign(oldRows.style, { position: 'absolute', top: '0', left: '0', pointerEvents: 'none' })
      screen.append(oldRows)
      const old = oldRows
      const timing = { duration: remote() ? 45 : 16, easing: 'linear' }
      old.animate([{ transform: 'translateY(0)' }, { transform: `translateY(${shift}px)` }], timing).onfinish = () => old.remove()
      rowAnimation = rows.animate([{ transform: `translateY(${-shift}px)` }, { transform: 'translateY(0)' }], timing)
    }
    previousRows = snapshot; scrollDelta = 0
  }
  function onStart(e: TouchEvent): void {
    cancel(); if (e.touches.length !== 1) return
    const f = e.touches[0], rect = screen.getBoundingClientRect()
    wheelOrigin = { x: Math.max(0, Math.min(1, (f.clientX - rect.left) / Math.max(1, rect.width))),
      y: Math.max(0, Math.min(1, (f.clientY - rect.top) / Math.max(1, rect.height))) }
    touch = { id: f.identifier, x: f.clientX, y: f.clientY, startY: f.clientY, active: false, time: performance.now(), speed: 0 }
  }
  function onMove(e: TouchEvent): void {
    if (!touch || e.touches.length !== 1) { cancel(); return }
    const f = Array.from(e.touches).find(t => t.identifier === touch!.id)
    if (!f) { cancel(); return }
    if (!touch.active) {
      if (Math.abs(f.clientY - touch.startY) < 5) return
      if (Math.abs(f.clientX - touch.x) > Math.abs(f.clientY - touch.startY)) { cancel(); return }
      touch.active = true
    }
    e.preventDefault(); e.stopPropagation()
    const delta = touch.y - f.clientY, now = performance.now()
    if (remote()) {
      const steps = Math.trunc(delta / (rowHeight() * 5))
      if (steps) { scrollLines(steps); touch.y -= steps * rowHeight() * 5 }
    } else if (term.buffer.active.type === 'normal') {
      localScroll(delta); touch.y = f.clientY
      touch.speed = .6 * touch.speed + .4 * Math.max(-2, Math.min(2, delta / Math.max(8, now - touch.time)))
    }
    touch.time = now
  }
  function onEnd(): void {
    if (touch?.active && !remote() && performance.now() - touch.time < 100) { inertia = touch.speed; schedule() }
    touch = null
  }
  function moveRail(e: PointerEvent): void {
    if (!drag || drag.id !== e.pointerId) return
    e.preventDefault()
    const rect = track.getBoundingClientRect(), ratio = Math.max(0, Math.min(1, (e.clientY - rect.top) / Math.max(1, rect.height)))
    if (remoteHistory && remote()) { remoteTarget = Math.round((1 - ratio) * remoteHistory.total); targetDeadline = performance.now() + 1500; schedule() }
    else if (!remote() && term.buffer.active.type === 'normal') term.scrollToLine(Math.round(ratio * term.buffer.active.baseY))
    else { const steps = Math.trunc((e.clientY - drag.y) / (rowHeight() * 5)); if (steps) { scrollLines(steps); drag.y = e.clientY } }
  }
  function startRail(e: PointerEvent): void {
    if (!e.isPrimary || e.button !== 0) return
    cancel(); drag = { id: e.pointerId, y: e.clientY }; track.setPointerCapture(e.pointerId)
    e.preventDefault(); e.stopPropagation(); moveRail(e)
  }
  const endRail = () => { drag = null }
  function key(e: KeyboardEvent): void {
    if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return
    e.preventDefault(); e.stopPropagation(); scrollLines(e.key === 'ArrowUp' ? -1 : 1)
  }
  screen.addEventListener('touchstart', onStart, { passive: true, capture: true })
  screen.addEventListener('touchmove', onMove, { passive: false, capture: true })
  screen.addEventListener('touchend', onEnd, true); screen.addEventListener('touchcancel', cancel, true)
  track.addEventListener('pointerdown', startRail); track.addEventListener('pointermove', moveRail)
  track.addEventListener('pointerup', endRail); track.addEventListener('pointercancel', cancel); track.addEventListener('lostpointercapture', endRail)
  track.addEventListener('keydown', key)
  window.addEventListener('blur', cancel); document.addEventListener('visibilitychange', cancel)
  const subscriptions = [term.onScroll(update), term.onWriteParsed(update), term.onResize(fit), term.onRender(render)]
  const observer = new ResizeObserver(fit)
  observer.observe(screen); observer.observe(host)
  update()
  return { reset: () => { cancel(); remoteHistory = null; previousPosition = null; previousRows = null; scrollDelta = 0 }, fit, dispose: () => {
    if (disposed) return
    cancel(); disposed = true
    screen.removeEventListener('touchstart', onStart, true); screen.removeEventListener('touchmove', onMove, true)
    screen.removeEventListener('touchend', onEnd, true); screen.removeEventListener('touchcancel', cancel, true)
    track.removeEventListener('pointerdown', startRail); track.removeEventListener('pointermove', moveRail)
    track.removeEventListener('pointerup', endRail); track.removeEventListener('pointercancel', cancel); track.removeEventListener('lostpointercapture', endRail)
    track.removeEventListener('keydown', key); window.removeEventListener('blur', cancel); document.removeEventListener('visibilitychange', cancel)
    observer.disconnect(); subscriptions.forEach(s => s.dispose()); rail.remove(); host.classList.remove('mobile-terminal')
  } }
}
