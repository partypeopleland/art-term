import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  DEFAULT_PANE_HOTKEYS,
  bindingFromEvent,
  hotkeyLabel,
  paneHotkeyAction,
  paneHotkeys,
  resetPaneHotkeys,
  setPaneHotkey,
  validatePaneHotkey,
  type HotkeyBinding
} from '../src/shared/state/hotkeys'

const bind = (partial: Partial<HotkeyBinding>): HotkeyBinding => ({
  ctrl: false, alt: false, shift: false, meta: false, code: 'KeyA', ...partial
})

beforeEach(() => resetPaneHotkeys())
afterEach(() => resetPaneHotkeys())

describe('pane hotkey bindings', () => {
  it('ships the issue #40 defaults and labels them by position', () => {
    expect(paneHotkeys.splitRight).toEqual({ ctrl: false, alt: true, shift: true, meta: false, code: 'Equal' })
    expect(paneHotkeys.splitDown).toEqual({ ctrl: false, alt: true, shift: true, meta: false, code: 'Minus' })
    expect(paneHotkeys.closePane).toEqual({ ctrl: false, alt: true, shift: true, meta: false, code: 'KeyX' })
    expect(hotkeyLabel(paneHotkeys.splitRight)).toBe('Alt+Shift++')
    expect(hotkeyLabel(paneHotkeys.splitDown)).toBe('Alt+Shift+-')
    expect(hotkeyLabel(paneHotkeys.closePane)).toBe('Alt+Shift+X')
  })

  it('matches on code and exact modifiers, ignoring key repeat', () => {
    const event = (init: KeyboardEventInit) => new KeyboardEvent('keydown', init)
    expect(paneHotkeyAction(event({ code: 'Equal', key: '+', altKey: true, shiftKey: true }))).toBe('splitRight')
    expect(paneHotkeyAction(event({ code: 'Equal', key: '=', altKey: true }))).toBeNull()
    expect(paneHotkeyAction(event({ code: 'Equal', key: '+', altKey: true, shiftKey: true, ctrlKey: true }))).toBeNull()
    expect(paneHotkeyAction(event({ code: 'Equal', key: '+', altKey: true, shiftKey: true, repeat: true }))).toBeNull()
    expect(paneHotkeyAction(event({ code: 'Minus', key: '_', altKey: true, shiftKey: true }))).toBe('splitDown')
    expect(paneHotkeyAction(event({ code: 'KeyX', key: 'X', altKey: true, shiftKey: true }))).toBe('closePane')
  })

  it('persists remaps per device and reloads them over the defaults', () => {
    setPaneHotkey('closePane', bind({ alt: true, shift: true, code: 'KeyQ' }))
    expect(JSON.parse(localStorage.getItem('art-term.ui.pane-hotkeys')!)).toMatchObject({
      closePane: { code: 'KeyQ' }
    })
    resetPaneHotkeys()
    expect(paneHotkeys.closePane.code).toBe('KeyX')
    expect(localStorage.getItem('art-term.ui.pane-hotkeys')).toBeNull()
  })

  it('rejects combos the real event path can never deliver to pane actions', () => {
    const cases: HotkeyBinding[] = [
      // isSwitchKey ignores Shift: Alt+[/] and Ctrl+PgUp/PgDn switch tabs first.
      bind({ alt: true, code: 'BracketLeft' }),
      bind({ alt: true, shift: true, code: 'BracketRight' }),
      bind({ ctrl: true, code: 'PageDown' }),
      bind({ ctrl: true, shift: true, code: 'PageUp' }),
      // isPaneKey owns these inside split views.
      bind({ ctrl: true, alt: true, code: 'ArrowLeft' }),
      bind({ ctrl: true, shift: true, code: 'KeyW' }),
      // Desktop clipboard eats Ctrl+C and Alt+C/Alt+V before handleKey.
      bind({ ctrl: true, code: 'KeyC' }),
      bind({ alt: true, code: 'KeyC' }),
      bind({ alt: true, code: 'KeyV' }),
      // No usable modifier at all.
      bind({ code: 'KeyA' }),
      bind({ shift: true, code: 'KeyA' })
    ]
    for (const candidate of cases) {
      expect(validatePaneHotkey('splitRight', candidate)).toBeTruthy()
    }
    const allowed: HotkeyBinding[] = [
      bind({ ctrl: true, shift: true, code: 'KeyC' }),
      bind({ alt: true, shift: true, code: 'KeyV' }),
      bind({ meta: true, code: 'BracketLeft' }),
      bind({ ctrl: true, alt: true, shift: true, code: 'ArrowRight' }),
      bind({ ctrl: true, alt: true, shift: true, code: 'KeyR' })
    ]
    for (const candidate of allowed) {
      expect(validatePaneHotkey('splitRight', candidate)).toBeNull()
    }
  })

  it('rejects duplicates across actions but allows keeping the current one', () => {
    setPaneHotkey('splitDown', bind({ ctrl: true, alt: false, shift: false, meta: false, code: 'KeyR' }))
    const clash = bind({ ctrl: true, code: 'KeyR' })
    expect(validatePaneHotkey('splitRight', clash)).toContain('向下分割')
    // Re-validating splitDown's own current binding is not a self-collision.
    expect(validatePaneHotkey('splitDown', clash)).toBeNull()
    // Defaults stay unique.
    for (const id of ['splitRight', 'splitDown', 'closePane'] as const) {
      expect(validatePaneHotkey(id, DEFAULT_PANE_HOTKEYS[id])).toBeNull()
    }
  })

  it('builds bindings from keydown and skips bare modifiers', () => {
    expect(bindingFromEvent(new KeyboardEvent('keydown', { code: 'ShiftLeft', shiftKey: true }))).toBeNull()
    expect(bindingFromEvent(new KeyboardEvent('keydown', { code: 'AltRight', altKey: true }))).toBeNull()
    const binding = bindingFromEvent(new KeyboardEvent('keydown', { code: 'Equal', key: '+', altKey: true, shiftKey: true }))
    expect(binding).toEqual({ ctrl: false, alt: true, shift: true, meta: false, code: 'Equal' })
  })
})
