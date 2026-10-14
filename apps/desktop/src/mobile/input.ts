// Touch keys and IME adaptation; sessions belong to the shared controller.
import { ref } from 'vue'
import type { Ref } from 'vue'
import type { TerminalHandle } from '../terminal/handle'

export type SpecialKey =
  | 'esc'
  | 'tab'
  | 'up'
  | 'down'
  | 'left'
  | 'right'
  | 'shiftleft'
  | 'ctrlc'
  | 'detach'

export const SPECIAL_BYTES: Record<Exclude<SpecialKey, 'detach'>, string> = {
  esc: '\x1b',
  tab: '\t',
  up: '\x1b[A',
  down: '\x1b[B',
  right: '\x1b[C',
  left: '\x1b[D',
  shiftleft: '\x1b[1;2D',
  ctrlc: '\x03'
}

/** Ctrl modifier mapping for a single printable char (termios-style). */
function applyCtrl(text: string): string {
  if (text.length !== 1) return text
  const code = text.charCodeAt(0)
  if (code === 0x3f) return '\x7f' // Ctrl+? = DEL
  if (code >= 0x20 && code <= 0x7f) return String.fromCharCode(code & 0x1f)
  return text
}

export interface MobileInput {
  /** Latched on-screen Ctrl: single chars become control bytes until toggled. */
  ctrlLatch: Ref<boolean>
  toggleCtrl(): void
  /** Special-key byte sequences; 'detach' has no bytes (local close only). */
  bytesFor(key: Exclude<SpecialKey, 'detach'>): string
  /** Backgrounding discards composition state; foreground never replays it. */
  setSuspended(value: boolean): void
  /**
   * Wrap a real terminal handle so the shared controller sees IME-safe,
   * latch-aware input. Composition fragments emitted through onData while a
   * DOM composition is active are held; compositionend (bubble phase, after
   * xterm's own commit) sends the authoritative text exactly once.
   */
  wrap(el: HTMLElement, real: TerminalHandle): TerminalHandle
}

export function createMobileInput(): MobileInput {
  const ctrlLatch = ref(false)
  let suspended = false
  const discarders = new Set<() => void>()

  function setSuspended(value: boolean): void {
    suspended = value
    if (value) {
      ctrlLatch.value = false
      for (const discard of discarders) discard()
    }
  }

  function toggleCtrl(): void {
    ctrlLatch.value = !ctrlLatch.value
  }

  function bytesFor(key: Exclude<SpecialKey, 'detach'>): string {
    return SPECIAL_BYTES[key]
  }

  function wrap(el: HTMLElement, real: TerminalHandle): TerminalHandle {
    let cb: ((text: string) => void) | null = null
    let innerInterceptor: ((ev: KeyboardEvent) => boolean) | null = null
    let imeActive = false
    let imeHeld: string[] = []
    let pendingCommit = ''
    let commitTimer: ReturnType<typeof setTimeout> | null = null
    let disposed = false
    let discardedComposition = false

    const clearCommit = (): void => {
      pendingCommit = ''
      if (commitTimer !== null) clearTimeout(commitTimer)
      commitTimer = null
    }

    const emit = (text: string): void => {
      if (!cb || suspended || disposed) return
      cb(ctrlLatch.value ? applyCtrl(text) : text)
    }

    const onCompStart = (): void => {
      clearCommit()
      discardedComposition = suspended
      imeActive = true
      imeHeld = []
    }
    const onCompEnd = (ev: Event): void => {
      imeActive = false
      if (discardedComposition || suspended) {
        imeHeld = []
        clearCommit()
        return
      }
      const held = imeHeld.join('')
      imeHeld = []
      const commit = (ev as CompositionEvent).data || held
      if (commit) {
        // xterm 6 finalizes composition in a zero-delay task after this
        // bubbling event. Suppress that one echoed commit, preserving any
        // following bytes (for example Enter). A new composition or task
        // clears this guard so intentional repeated text remains valid.
        clearCommit()
        pendingCommit = commit
        commitTimer = setTimeout(clearCommit, 0)
        emit(commit)
      }
    }
    el.addEventListener('compositionstart', onCompStart)
    el.addEventListener('compositionend', onCompEnd)
    const discard = (): void => {
      discardedComposition ||= imeActive || Boolean(pendingCommit)
      imeHeld = []
      clearCommit()
    }
    // A canceled composition's delayed xterm callback is indistinguishable
    // from ordinary onData. Only fresh user intent releases its fence.
    const onFreshInput = (ev: Event): void => {
      if (!suspended && !imeActive && !(ev instanceof KeyboardEvent && ev.isComposing)) discardedComposition = false
    }
    discarders.add(discard)
    el.addEventListener('keydown', onFreshInput, true)
    el.addEventListener('paste', onFreshInput, true)

    // The wrapped handle's own interceptor runs first: a latched Ctrl +
    // single-char hardware keydown is converted and swallowed so xterm does
    // not also emit the raw char; everything else defers to the controller's
    // interceptor (tab-switch keys etc.) before xterm sees the event.
    real.setKeyInterceptor((ev) => {
      if (ev.type !== 'keydown') return innerInterceptor ? innerInterceptor(ev) : true
      if (
        ctrlLatch.value &&
        !ev.ctrlKey &&
        !ev.altKey &&
        !ev.metaKey &&
        ev.key.length === 1
      ) {
        emit(ev.key)
        return false
      }
      return innerInterceptor ? innerInterceptor(ev) : true
    })
    real.onData((text) => {
      if (disposed || suspended || discardedComposition) return
      if (imeActive) {
        imeHeld.push(text)
        return
      }
      if (pendingCommit) {
        const commit = pendingCommit
        if (text.startsWith(commit)) {
          clearCommit()
          const remainder = text.slice(commit.length)
          if (remainder) emit(remainder)
          return
        }
        // A separate Enter/key event may precede xterm's queued commit.
        // Keep the guard for the matching echo until the task window ends.
      }
      emit(text)
    })

    return {
      get rows() {
        return real.rows
      },
      get cols() {
        return real.cols
      },
      bracketedPasteMode: () => real.bracketedPasteMode?.() ?? false,
      readStyledText: () => real.readStyledText?.() ?? { records: [], supported: false },
      setNativeReaderActive: active => real.setNativeReaderActive?.(active),
      write: (data, historical) => real.write(data, historical),
      onData: (fn) => {
        cb = fn
      },
      setKeyInterceptor: (fn) => {
        innerInterceptor = fn
      },
      fit: () => real.fit(),
      focus: () => real.focus(),
      suppressSoftKeyboard: () => real.suppressSoftKeyboard?.(),
      reset: () => real.reset(),
      dispose: () => {
        if (disposed) return
        disposed = true
        clearCommit()
        imeHeld = []
        discarders.delete(discard)
        cb = null
        innerInterceptor = null
        el.removeEventListener('compositionstart', onCompStart)
        el.removeEventListener('compositionend', onCompEnd)
        el.removeEventListener('keydown', onFreshInput, true)
        el.removeEventListener('paste', onFreshInput, true)
        real.dispose()
      }
    }
  }

  return { ctrlLatch, toggleCtrl, bytesFor, wrap, setSuspended }
}
