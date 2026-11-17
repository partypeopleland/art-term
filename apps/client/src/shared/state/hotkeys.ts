import { reactive } from 'vue'

/**
 * Configurable pane hotkeys for the Windows split view. Bindings match on
 * `KeyboardEvent.code` (physical key position), never on `event.key`, so
 * Shift+= is recognised as `Equal` and produces the same split regardless of
 * the shifted character. Defaults follow issue #40: Shift+Alt+= splits right
 * (the `+` key is Shift+= on common layouts), Shift+Alt+- splits down and
 * Shift+Alt+X closes the focused pane.
 */
export type PaneHotkeyAction = 'splitRight' | 'splitDown' | 'closePane'

export interface HotkeyBinding {
  ctrl: boolean
  alt: boolean
  shift: boolean
  meta: boolean
  code: string
}

export const PANE_HOTKEY_ACTIONS: readonly { id: PaneHotkeyAction; label: string }[] = [
  { id: 'splitRight', label: '向右分割' },
  { id: 'splitDown', label: '向下分割' },
  { id: 'closePane', label: '關閉窗格' }
]

export const DEFAULT_PANE_HOTKEYS: Record<PaneHotkeyAction, HotkeyBinding> = {
  splitRight: { ctrl: false, alt: true, shift: true, meta: false, code: 'Equal' },
  splitDown: { ctrl: false, alt: true, shift: true, meta: false, code: 'Minus' },
  closePane: { ctrl: false, alt: true, shift: true, meta: false, code: 'KeyX' }
}

const STORAGE_KEY = 'art-term.ui.pane-hotkeys'

function validBinding(value: unknown): value is HotkeyBinding {
  if (!value || typeof value !== 'object') return false
  const b = value as Record<string, unknown>
  return typeof b.code === 'string' && b.code.length > 0 && b.code.length <= 32
    && typeof b.ctrl === 'boolean' && typeof b.alt === 'boolean'
    && typeof b.shift === 'boolean' && typeof b.meta === 'boolean'
}

function loadHotkeys(): Record<PaneHotkeyAction, HotkeyBinding> {
  const loaded = { ...DEFAULT_PANE_HOTKEYS }
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return loaded
    const parsed = JSON.parse(raw) as Record<string, unknown>
    for (const { id } of PANE_HOTKEY_ACTIONS) {
      const candidate = parsed?.[id]
      if (validBinding(candidate)) loaded[id] = { ...candidate }
    }
  } catch { /* Keep the defaults when storage is unavailable or corrupt. */ }
  return loaded
}

export const paneHotkeys = reactive<Record<PaneHotkeyAction, HotkeyBinding>>(loadHotkeys())

export function setPaneHotkey(action: PaneHotkeyAction, binding: HotkeyBinding): void {
  paneHotkeys[action] = { ...binding }
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(paneHotkeys)) } catch { /* In-memory only. */ }
}

export function resetPaneHotkeys(): void {
  for (const { id } of PANE_HOTKEY_ACTIONS) paneHotkeys[id] = { ...DEFAULT_PANE_HOTKEYS[id] }
  try { localStorage.removeItem(STORAGE_KEY) } catch { /* Ignore. */ }
}

export function hotkeyMatches(event: KeyboardEvent, binding: HotkeyBinding): boolean {
  return event.type === 'keydown'
    && event.code === binding.code
    && event.ctrlKey === binding.ctrl
    && event.altKey === binding.alt
    && event.shiftKey === binding.shift
    && event.metaKey === binding.meta
}

/** The action a keydown triggers under the current mapping, or null. */
export function paneHotkeyAction(event: KeyboardEvent): PaneHotkeyAction | null {
  if (event.repeat) return null
  for (const { id } of PANE_HOTKEY_ACTIONS) {
    if (hotkeyMatches(event, paneHotkeys[id])) return id
  }
  return null
}

const MODIFIER_CODES = new Set(['ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight'])

/** Build a binding candidate from a keydown; null for bare modifier presses. */
export function bindingFromEvent(event: KeyboardEvent): HotkeyBinding | null {
  if (MODIFIER_CODES.has(event.code)) return null
  return {
    ctrl: event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
    meta: event.metaKey,
    code: event.code
  }
}

function sameBinding(a: HotkeyBinding, b: HotkeyBinding): boolean {
  return a.code === b.code && a.ctrl === b.ctrl && a.alt === b.alt && a.shift === b.shift && a.meta === b.meta
}

/** Combos already owned by tab switching, pane focus cycling, pane close or clipboard. */
function isReservedBinding(binding: HotkeyBinding): boolean {
  const bare = !binding.shift && !binding.meta
  if (bare && binding.alt && !binding.ctrl && (binding.code === 'BracketLeft' || binding.code === 'BracketRight')) return true
  if (bare && binding.ctrl && !binding.alt && (binding.code === 'PageUp' || binding.code === 'PageDown')) return true
  if (binding.ctrl && binding.alt && !binding.shift && binding.code.startsWith('Arrow')) return true
  if (binding.ctrl && binding.shift && !binding.alt && binding.code === 'KeyW') return true
  if (bare && binding.alt && !binding.ctrl && (binding.code === 'KeyC' || binding.code === 'KeyV')) return true
  return false
}

/**
 * Validate a candidate binding for `action` against the current map.
 * Returns a user-facing reason when rejected, null when accepted.
 */
export function validatePaneHotkey(
  action: PaneHotkeyAction,
  binding: HotkeyBinding,
  current: Record<PaneHotkeyAction, HotkeyBinding> = paneHotkeys
): string | null {
  if (!binding.ctrl && !binding.alt && !binding.meta) return '需要搭配 Ctrl、Alt 或 Meta 修飾鍵。'
  if (isReservedBinding(binding)) return '此組合已被其他快捷鍵使用。'
  for (const { id, label } of PANE_HOTKEY_ACTIONS) {
    if (id !== action && sameBinding(binding, current[id])) return `此組合已指派給「${label}」。`
  }
  return null
}

const CODE_LABELS: Record<string, string> = {
  Equal: '=', Minus: '-', BracketLeft: '[', BracketRight: ']', Semicolon: ';',
  Quote: "'", Comma: ',', Period: '.', Slash: '/', Backquote: '`', Backslash: '\\',
  Space: 'Space', Tab: 'Tab', Enter: 'Enter', Backspace: 'Backspace', Escape: 'Esc',
  ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→'
}

function keyLabel(binding: HotkeyBinding): string {
  const code = binding.code
  let label = CODE_LABELS[code]
  if (!label) {
    const named = /^Key([A-Z])$/.exec(code)
    if (named) label = named[1]
    else {
      const digit = /^Digit([0-9])$/.exec(code)
      if (digit) label = digit[1]
      else label = code
    }
  }
  // `+` only exists as Shift+= on common layouts; show the produced character
  // for the default split-right binding while matching stays code-based.
  if (binding.shift && code === 'Equal') return '+'
  return label
}

/** Display form like `Shift+Alt+=`; modifiers in Ctrl/Alt/Shift/Meta order. */
export function hotkeyLabel(binding: HotkeyBinding): string {
  const parts: string[] = []
  if (binding.ctrl) parts.push('Ctrl')
  if (binding.alt) parts.push('Alt')
  if (binding.shift) parts.push('Shift')
  if (binding.meta) parts.push('Meta')
  parts.push(keyLabel(binding))
  return parts.join('+')
}
