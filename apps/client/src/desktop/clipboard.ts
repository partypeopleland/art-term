import type { TabState } from '../shared/state/controller'

interface DesktopClipboardDeps {
  activeTab(): TabState | null
  readText(): Promise<string>
  writeText(text: string): Promise<void>
  send(tab: TabState, text: string): void
  notice(message: string): void
}

/** Clipboard handling is intentionally scoped to the Windows terminal UI. */
export function createDesktopClipboard(deps: DesktopClipboardDeps) {
  const lastAutoCopy = new WeakMap<TabState, string>()
  const isLive = (tab: TabState | null): tab is TabState =>
    !!tab && !tab.closed && tab.status === 'live' && !!tab.sessionId && !!tab.term

  async function copy(tab: TabState): Promise<void> {
    const text = tab.term?.getSelection?.() ?? ''
    if (!text) return
    try {
      await deps.writeText(text)
      if (isLive(tab) && deps.activeTab() === tab) deps.notice('已複製')
    } catch {
      lastAutoCopy.delete(tab)
      deps.notice('無法複製到剪貼簿')
    }
  }

  async function selectionFinished(tab: TabState): Promise<void> {
    if (!isLive(tab) || deps.activeTab() !== tab) return
    const text = tab.term!.getSelection?.() ?? ''
    if (!text) { lastAutoCopy.delete(tab); return }
    if (lastAutoCopy.get(tab) === text) return
    lastAutoCopy.set(tab, text)
    await copy(tab)
  }

  async function paste(tab: TabState): Promise<void> {
    let text: string
    try { text = await deps.readText() }
    catch { deps.notice('無法讀取剪貼簿'); return }
    if (!text || !isLive(tab) || deps.activeTab() !== tab) return
    // Terminal input is a byte stream. Outside bracketed paste, CR/LF can
    // execute a shell command; control codes can also escape paste framing.
    if (text.includes('\x1b') || text.includes('\0') || text.includes('\x7f') || /[\x01-\x08\x0b-\x0c\x0e-\x1f]/.test(text)) {
      deps.notice('剪貼簿含無法安全貼上的控制字元')
      return
    }
    if (/[\r\n\t]/.test(text)) {
      if (!tab.term!.bracketedPasteMode?.()) {
        // GNU Readline's quoted-insert (Ctrl+V) inserts the following
        // newline into the editing buffer rather than accepting the line.
        // Other remote programs may interpret this byte stream differently.
        deps.send(tab, text.replace(/[\r\n\t]/g, character => `\x16${character}`))
      } else {
        deps.send(tab, `\x1b[200~${text}\x1b[201~`)
      }
    } else {
      deps.send(tab, text)
    }
  }

  function handleKey(event: KeyboardEvent): boolean {
    if (event.type !== 'keydown' || event.metaKey || event.shiftKey) return false
    const tab = deps.activeTab()
    if (!isLive(tab)) return false
    if (event.ctrlKey && !event.altKey && event.code === 'KeyC') {
      event.preventDefault()
      event.stopPropagation()
      deps.send(tab, '\x03')
      return true
    }
    if (!event.altKey || event.ctrlKey || (event.code !== 'KeyC' && event.code !== 'KeyV')) return false
    event.preventDefault()
    event.stopPropagation()
    if (event.code === 'KeyC') void copy(tab)
    else void paste(tab)
    return true
  }

  return { handleKey, selectionFinished }
}
