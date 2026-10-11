// Mobile Tauri adapter — the ONLY mobile module that imports @tauri-apps/api.
// Only registered runtime-session commands cross this boundary.
import { invoke } from '@tauri-apps/api/core'
import { asConnectionError } from '../transport/types'
import type {
  TransferPreview, TransferResult,
  DesktopStateDto,
  HostDto,
  OutputDto,
  RuntimeHostDto,
  RuntimeHostTestDto,
  TargetDto,
  SessionDto,
  ScreenshotDto,
  Transport
} from '../transport/types'

function sanitizeError(e: unknown): string {
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

export function createMobileTauriTransport(): Transport {
  return {
    transferExport: input => call('gui_transfer_export', { input }),
    transferPickImport: () => call<TransferPreview>('gui_transfer_pick_import'),
    transferUnlock: (token, password) => call<TransferPreview>('gui_transfer_unlock', { token, password }),
    // Snapshot one-shot inputs synchronously before Vue clears its references.
    transferApply: (token, choices) => call<TransferResult>('gui_transfer_apply', { token, choices: choices.map(choice => ({ ...choice })) }),
    transferCancel: () => call('gui_transfer_cancel'),
    runtimeAck: (sessionId, batchId, localGeneration) => call<void>('gui_runtime_ack', { sessionId, batchId, localGeneration }),
    runtimeRecover: (sessionId) => call<void>('gui_runtime_recover', { sessionId }),
    runtimeSessions: (id, expectedRevision) => call<HostDto>('gui_runtime_sessions', { id, expectedRevision }),
    runtimeCreateSession: (id, expectedRevision, name) => call<TargetDto>('gui_runtime_create_session', { id, expectedRevision, name }),
    runtimeHosts: () => call<RuntimeHostDto[]>('gui_runtime_hosts'),
    runtimeSaveHost: (input) => call<RuntimeHostDto>('gui_runtime_save_host', { input }),
    runtimeDeleteHost: (id, expectedRevision) => call<void>('gui_runtime_delete_host', { id, expectedRevision }),
    runtimeTestHost: (id, expectedRevision) => call<RuntimeHostTestDto>('gui_runtime_test_host', { id, expectedRevision }),
    // --- shared Transport contract over backend-issued target ids ---
    open: (targetId, rows, cols) =>
      call<SessionDto>('mobile_open', { targetId, rows, cols }),
    poll: (sessionId) => call<OutputDto>('mobile_poll', { sessionId }),
    write: (sessionId, bytes) => call<void>('mobile_write', { sessionId, bytes }),
    resize: (sessionId, rows, cols) =>
      call<void>('mobile_resize', { sessionId, rows, cols }),
    close: (sessionId) => call<void>('mobile_close', { sessionId }),
    desktop: (sessionId, enabled) =>
      call<DesktopStateDto>('mobile_desktop', { sessionId, enabled }),
    screenshot: (sessionId) => call<ScreenshotDto>('mobile_screenshot', { sessionId })
  }
}
