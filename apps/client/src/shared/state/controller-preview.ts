import type { ComputedRef } from 'vue'
import type { InstanceDesktop, Transport } from '../transport/types'
import type { TabState } from './controller-types'
import { errText } from './controller-errors'

const SAFE_IMAGE_MIME = /^image\/(png|jpeg|webp|gif|bmp)$/i

/** Owns preview scheduling, Screen selection fences, and image validity. */
export function createPreviewController(
  transport: Transport,
  activeTab: ComputedRef<TabState | null>,
  instanceOperations: Map<string, number>,
  autoCaptureMs: number,
  isDisposed: () => boolean,
  isHidden: () => boolean
) {
  // --- desktop / screenshot scheduling ---

  let captureBusy = false
  let capturePending: TabState | null = null

  function canCapture(tab: TabState | null | undefined): tab is TabState {
    return (
      !!tab &&
      !isDisposed() &&
      !isHidden() &&
      !tab.closed &&
      tab.status === 'live' &&
      !!tab.sessionId &&
      tab.desktopEnabled &&
      tab.previewVisible &&
      tab === activeTab.value
    )
  }

  function invalidateCapture(tab: TabState): void {
    tab.captureGen++
    if (capturePending === tab) capturePending = null
  }

  function instanceOperation(tab: TabState): number {
    const next = (instanceOperations.get(tab.key) ?? 0) + 1
    instanceOperations.set(tab.key, next)
    invalidateCapture(tab)
    tab.shot = null
    tab.shotError = ''
    return next
  }
  async function selectInstance(tab: TabState, desktop: InstanceDesktop): Promise<void> {
    if (!transport.instanceSelect || isHidden() || isDisposed() || tab.closed || tab.status !== 'live' || !tab.sessionId) return
    const sid = tab.sessionId, operation = instanceOperation(tab)
    tab.desktopBusy = true; tab.desktopError = ''; tab.desktopEnabled = false
    tab.instanceDesktop = null; tab.instanceToolReady = false
    try {
      const result = await transport.instanceSelect(sid, desktop.id, desktop.revision)
      if (isDisposed() || isHidden() || tab.closed || tab.sessionId !== sid || instanceOperations.get(tab.key) !== operation) return
      if (result.desktop.id !== desktop.id || result.desktop.revision !== desktop.revision) throw new Error('Screen 設定已變更，請重新選擇')
      tab.instanceDesktop = { ...result.desktop }; tab.instanceToolReady = result.toolReady
      tab.desktopBusy = false
      await setDesktop(tab, true)
      if (instanceOperations.get(tab.key) === operation) tab.desktopMsg = ''
    } catch {
      if (!tab.closed && tab.sessionId === sid && instanceOperations.get(tab.key) === operation) tab.desktopError = 'Screen 選擇失敗，請確認設定與連線後重試'
    } finally {
      if (instanceOperations.get(tab.key) === operation) tab.desktopBusy = false
    }
  }
  async function clearInstance(tab: TabState): Promise<void> {
    const sid = tab.sessionId, operation = instanceOperation(tab)
    tab.instanceDesktop = null; tab.instanceToolReady = false; tab.desktopEnabled = false
    tab.desktopMsg = ''; tab.desktopError = ''; tab.desktopBusy = true
    try { if (sid) await transport.instanceClear?.(sid) }
    catch { if (!tab.closed && tab.sessionId === sid && instanceOperations.get(tab.key) === operation) tab.desktopError = '預覽已停止；遠端工具取消尚未確認，請重試或結束 CLI' }
    finally { if (instanceOperations.get(tab.key) === operation) tab.desktopBusy = false }
  }

  /** Serialized toggles; a response landing on a closed/replaced tab is dropped. */
  async function setDesktop(tab: TabState, enabled: boolean): Promise<void> {
    if (isHidden() || tab.desktopBusy || tab.closed || tab.status !== 'live' || !tab.sessionId) return
    tab.desktopBusy = true
    tab.desktopError = ''
    const sid = tab.sessionId, captureVersion = tab.captureGen, operation = instanceOperations.get(tab.key)
    try {
      const st = await transport.desktop(sid, enabled)
      if (isHidden() || tab.closed || tab.sessionId !== sid || tab.captureGen !== captureVersion) return
      tab.desktopEnabled = st.enabled
      tab.desktopMsg = st.message
      if (st.enabled) {
        tab.previewVisible = true
        if (tab === activeTab.value) requestCapture(tab)
      } else {
        invalidateCapture(tab)
        // A disabled preview must not keep showing the last frame.
        tab.shot = null
        tab.shotError = ''
      }
    } catch (e) {
      if (!tab.closed && tab.sessionId === sid) tab.desktopError = errText(e)
    } finally {
      if (instanceOperations.get(tab.key) === operation) tab.desktopBusy = false
    }
  }

  function setPreviewVisible(tab: TabState, visible: boolean): void {
    tab.previewVisible = visible
    if (visible) {
      if (tab.sessionId && tab.desktopEnabled) void transport.desktop(tab.sessionId, true).then(() => { if (canCapture(tab)) requestCapture(tab) }).catch(() => undefined)
    } else {
      invalidateCapture(tab)
      tab.shot = null
      if (tab.sessionId && (tab.desktopEnabled || tab.desktopBusy)) void transport.desktop(tab.sessionId, false).catch(() => undefined)
    }
  }

  function requestCapture(tab: TabState): void {
    if (!canCapture(tab)) return
    if (captureBusy) {
      // One global in-flight capture; manual/auto requests coalesce latest-wins.
      capturePending = tab
      return
    }
    void runCapture(tab)
  }

  async function runCapture(tab: TabState): Promise<void> {
    captureBusy = true
    const sid = tab.sessionId!
    const gen = ++tab.captureGen
    try {
      const s = await transport.screenshot(sid)
      if (
        !tab.closed &&
        tab.sessionId === sid &&
        tab.captureGen === gen &&
        tab.desktopEnabled &&
        tab.previewVisible &&
        tab === activeTab.value &&
        (!tab.instanceDesktop || (s.desktopId === tab.instanceDesktop.id && s.desktopRevision === tab.instanceDesktop.revision)) &&
        SAFE_IMAGE_MIME.test(s.mime)
      ) {
        tab.shot = { src: `data:${s.mime};base64,${s.base64}`, source: s.source, capturedAt: s.capturedAt }
        tab.shotError = ''
      }
    } catch (e) {
      if (!tab.closed && tab.sessionId === sid && tab.captureGen === gen) {
        // Keep this Screen's last successful frame, but mark it stale.
        tab.shotError = errText(e)
      }
    } finally {
      captureBusy = false
      const pending = capturePending
      capturePending = null
      if (pending && canCapture(pending)) void runCapture(pending)
    }
  }

  function manualCapture(): void {
    const tab = activeTab.value
    if (tab && canCapture(tab)) requestCapture(tab)
  }

  // First capture is immediate (on enable / activate / show), then every
  // autoCaptureMs for the active tab only. Shell input is never blocked.
  const captureTimer = setInterval(() => {
    const tab = activeTab.value
    if (tab && canCapture(tab)) requestCapture(tab)
  }, autoCaptureMs)


  function disposePreview(): void {
    clearInterval(captureTimer)
    capturePending = null
  }

  return { canCapture, invalidateCapture, requestCapture, selectInstance, clearInstance, setDesktop, setPreviewVisible, manualCapture, disposePreview }
}
