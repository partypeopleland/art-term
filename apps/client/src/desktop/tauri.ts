// Dedicated Tauri adapter — the ONLY module that imports @tauri-apps/api.
// Vue components and the controller depend only on the Transport interface,
// so this adapter can later be replaced by HTTP/WebSocket without UI changes.
import { invoke } from '@tauri-apps/api/core'
import { asConnectionError } from '../shared/transport/types'
import type {
  TransferPreview, TransferResult, InstanceDesktop, InstanceSelection,
  DesktopStateDto,
  HostDto,
  OutputDto,
  RuntimeHostDto,
  RuntimeHostTestDto,
  SessionDto,
  ScreenshotDto,
  Transport,
  TargetDto
} from '../shared/transport/types'

export function sanitizeError(e: unknown): string {
  let msg: string
  if (typeof e === 'string') msg = e
  else if (e instanceof Error) msg = e.message
  else {
    try {
      msg = String(e)
    } catch {
      msg = '未知錯誤'
    }
  }
  const oneLine = msg.replace(/\s+/g, ' ').trim()
  return oneLine.length > 300 ? `${oneLine.slice(0, 300)}…` : oneLine
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args)
  } catch (e) {
    // Structured {kind,message} rejections keep their kind; everything else
    // degrades to a sanitized string.
    const conn = asConnectionError(e)
    if (conn) throw conn
    throw new Error(sanitizeError(e))
  }
}

export function createTauriTransport(): Transport {
  return {
    instanceDesktops: () => call<InstanceDesktop[]>('gui_instance_desktops'),
    instanceSave: input => call<InstanceDesktop>('gui_instance_save', { input: { ...input } }),
    instanceDelete: (id, expectedRevision) => call('gui_instance_delete', { id, expectedRevision }),
    instanceTest: (id, expectedRevision) => call('gui_instance_test', { id, expectedRevision }),
    instanceSelect: (sessionId, desktopId, expectedRevision) => call<InstanceSelection>('gui_instance_select', { sessionId, desktopId, expectedRevision }),
    instanceRestore: sessionId => call<InstanceSelection | null>('gui_instance_restore', { sessionId }),
    instanceClear: sessionId => call('gui_instance_clear', { sessionId }),
    transferExport: input => call('gui_transfer_export', { input }),
    transferPickImport: () => call<TransferPreview>('gui_transfer_pick_import'),
    transferUnlock: (token, password) => call<TransferPreview>('gui_transfer_unlock', { token, password }),
    // Snapshot one-shot inputs synchronously before Vue clears its references.
    transferApply: (token, choices) => call<TransferResult>('gui_transfer_apply', { token, choices: choices.map(choice => ({ ...choice })) }),
    transferCancel: () => call('gui_transfer_cancel'),
    runtimeAck: (sessionId, batchId, localGeneration) => call<void>('gui_runtime_ack', { sessionId, batchId, localGeneration }),
    runtimeRecover: (sessionId) => call<void>('gui_runtime_recover', { sessionId }),
    runtimeSessions: (id, expectedRevision) => call<HostDto>('gui_runtime_sessions', { id, expectedRevision }),
    runtimeCreateHistorySession: (id, expectedRevision) => call('gui_runtime_create_history_session', { id, expectedRevision }),
    runtimeHistory: (sessionId, request) => call('gui_runtime_history', { sessionId, request }),
    runtimeCreateSession: (id, expectedRevision, name) => call<TargetDto>('gui_runtime_create_session', { id, expectedRevision, name }),
    runtimeHosts: () => call<RuntimeHostDto[]>('gui_runtime_hosts'),
    runtimeReorderConnections: (kind, expectedIds, orderedIds) => call<void>('gui_runtime_reorder_connections', { kind, expectedIds, orderedIds }),
    runtimeSaveHost: (input) => call<RuntimeHostDto>('gui_runtime_save_host', { input }),
    runtimeDeleteHost: (id, expectedRevision) => call<void>('gui_runtime_delete_host', { id, expectedRevision }),
    runtimeTestHost: (id, expectedRevision) => call<RuntimeHostTestDto>('gui_runtime_test_host', { id, expectedRevision }),
    open: (targetId, rows, cols) => call<SessionDto>('gui_open', { targetId, rows, cols }),
    poll: (sessionId) => call<OutputDto>('gui_poll', { sessionId }),
    write: (sessionId, bytes, terminalReply) => call<void>('gui_write', { sessionId, bytes, ...(terminalReply ? { terminalReply: true } : {}) }),
    resize: (sessionId, rows, cols) => call<void>('gui_resize', { sessionId, rows, cols }),
    close: (sessionId) => call<void>('gui_close', { sessionId }),
    desktop: (sessionId, enabled) => call<DesktopStateDto>('gui_desktop', { sessionId, enabled }),
    screenshot: (sessionId) => call<ScreenshotDto>('gui_screenshot', { sessionId })
  }
}
