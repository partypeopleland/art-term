import type { Transport } from '../transport/types'
import type { TabState } from './controller-types'
import { errText } from './controller-errors'

const WRITE_CAP = 256 * 1024
const WRITE_CHUNK = 16 * 1024
const textEncoder = new TextEncoder()

/** Serializes terminal writes, protocol replies, and latest-size updates per tab. */
export function createTerminalIo(
  transport: Transport,
  isDisposed: () => boolean,
  isHidden: () => boolean
) {
  const replyQueues = new WeakMap<TabState, { sid: string; generation: number; bytes: number[]; writing: boolean }>()
  function sendTerminalReply(tab: TabState, text: string): void {
    if (isDisposed() || tab.closed || tab.status !== 'live' || !tab.sessionId) return
    let queue = replyQueues.get(tab)
    if (!queue || queue.sid !== tab.sessionId || queue.generation !== tab.connectionGen) {
      queue = { sid: tab.sessionId, generation: tab.connectionGen, bytes: [], writing: false }
      replyQueues.set(tab, queue)
    }
    const bytes = Array.from(textEncoder.encode(text))
    if (queue.bytes.length + bytes.length > 16 * 1024) return
    queue.bytes.push(...bytes)
    if (queue.writing) return
    const owned = queue
    owned.writing = true
    void (async () => {
      try {
        while (owned.bytes.length && !tab.closed && !isDisposed() && tab.sessionId === owned.sid && tab.connectionGen === owned.generation) {
          const chunk = owned.bytes.splice(0, WRITE_CHUNK)
          await transport.write(owned.sid, chunk, true)
        }
      } catch { owned.bytes = [] }
      finally { owned.bytes = []; owned.writing = false }
    })()
  }

  function sendInput(tab: TabState, text: string): void {
    // A modal password must never fall through to a previously focused terminal.
    if (isHidden() || tab.status !== 'live' || !tab.sessionId || tab.closed) return
    const bytes = Array.from(textEncoder.encode(text))
    if (!bytes.length) return
    tab.errorMsg = ''
    if (tab.writeBuf.length + bytes.length > WRITE_CAP) {
      // Reject this whole input event; never truncate queued command prefixes
      // or retain only a pasted suffix that might contain an execution newline.
      tab.writeDropped = true
      tab.notice = '輸入佇列已滿，本次輸入未送出；請稍後重試'
      return
    } else {
      tab.writeBuf = tab.writeBuf.concat(bytes)
    }
    void flushWrite(tab)
  }

  async function flushWrite(tab: TabState): Promise<void> {
    if (tab.writing) return
    tab.writing = true
    const sid = tab.sessionId
    const generation = tab.connectionGen
    try {
      while (tab.writeBuf.length && tab.status === 'live' && sid && tab.sessionId === sid && tab.connectionGen === generation && !tab.closed) {
        const chunk = tab.writeBuf.splice(0, WRITE_CHUNK)
        try {
          await transport.write(sid, chunk)
        } catch (e) {
          if (tab.connectionGen === generation) {
            tab.errorMsg = `輸入未送出：${errText(e)}`
            tab.writeBuf = []
          }
          break
        }
      }
    } finally {
      tab.writing = false
      if (!tab.writeBuf.length) tab.writeDropped = false
      // A new connection may receive fresh input while an old write settles.
      // Its own queue is safe to flush; detach already discarded old input.
      if (tab.connectionGen !== generation && tab.writeBuf.length && tab.status === 'live') {
        void flushWrite(tab)
      }
    }
  }

  function requestResize(tab: TabState, rows: number, cols: number): void {
    if (!rows || !cols || tab.closed) return
    if (tab.lastSize && tab.lastSize.rows === rows && tab.lastSize.cols === cols) return
    tab.lastSize = { rows, cols }
    if (tab.status !== 'live' || !tab.sessionId) return
    tab.pendingSize = { rows, cols }
    void flushResize(tab)
  }

  async function flushResize(tab: TabState): Promise<void> {
    if (tab.resizing) return
    tab.resizing = true
    const sid = tab.sessionId
    const generation = tab.connectionGen
    try {
      // Latest-wins coalescing keeps this bounded and serialized.
      while (tab.pendingSize && tab.status === 'live' && sid && tab.sessionId === sid && tab.connectionGen === generation && !tab.closed) {
        const { rows, cols } = tab.pendingSize
        tab.pendingSize = null
        try {
          await transport.resize(sid, rows, cols)
        } catch (e) {
          if (tab.connectionGen === generation) tab.errorMsg = errText(e)
          break
        }
      }
    } finally {
      tab.resizing = false
      if (tab.connectionGen !== generation && tab.pendingSize && tab.status === 'live') {
        void flushResize(tab)
      }
    }
  }

  return { sendTerminalReply, sendInput, requestResize }
}
