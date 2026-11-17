import type { ComputedRef, Ref } from 'vue'
import type { InstanceDesktop, HostDto, RuntimeHostDto, TargetDto } from '../transport/types'
import type { TerminalHandle } from '../terminal/handle'
import type { HostHistoryReply, HostHistoryRequest } from '../terminal/native'
import type { PaneNode, SplitDirection } from '../terminal/split'

export type TabStatus = 'connecting' | 'live' | 'exited' | 'error'

export interface ShotState {
  src: string
  source: string
  capturedAt: number
}

export interface TabState {
  /** Stable key; target identity for reuse. */
  key: string
  targetId: string
  hostId: string
  hostLabel: string
  label: string
  status: TabStatus
  errorMsg: string
  notice: string
  sessionId: string | null
  remoteSession: string
  runtime: boolean
  /**
   * This tab's runtime Host profile is bound to a declared runtime_agents
   * entry (backend metadata, exact configured origin match — never a
   * label guess). Lifecycle-terminal runtime kinds retire the tab instead
   * of leaving a dead reconnect target: no automatic rebind to a
   * replacement runtime, pending input/resize die with the tab.
   */
  runtimeAgent: boolean
  exitCode: number | null
  instanceDesktop?: InstanceDesktop | null
  instanceToolReady?: boolean
  desktopAvailable: boolean
  desktopEnabled: boolean
  desktopBusy: boolean
  desktopMsg: string
  desktopError: string
  previewVisible: boolean
  shot: ShotState | null
  shotError: string
  captureGen: number
  /** Internal, markRaw'd once attached. */
  term: TerminalHandle | null
  /**
   * Split pane tree for the Windows view. Null = single connection. Every
   * `conn` leaf owns a full TabState registered in `tabs` with `paneOf`
   * pointing back at this tab's key; the leaf whose tab IS this tab is the
   * original connection. Any conn leaf — including the original — can be
   * closed individually while sibling conn panes survive (the first
   * survivor is promoted to own this tab slot).
   */
  paneRoot: PaneNode | null
  /** Leaf id of the pane that currently owns keyboard focus (`conn:${key}` style ids). */
  focusedLeafId: string | null
  /** Monotonic suffix for generated pane leaf ids. */
  paneSeq: number
  /** Set on pane child tabs: key of the owning split-view tab. */
  paneOf: string | null
  wantDesktop: boolean
  closed: boolean
  writeBuf: number[]
  writing: boolean
  writeDropped: boolean
  pendingSize: { rows: number; cols: number } | null
  resizing: boolean
  lastSize: { rows: number; cols: number } | null
  connectionGen: number
}

export interface HostUiState {
  dto: HostDto
  expanded: boolean
}

export interface ControllerOptions {
  idlePollMs?: number
  autoCaptureMs?: number
  backgroundConnection?: 'detach' | 'keep'
  /** Mobile returns to target selection when the remote Shell exits. */
  closeFinishedTabs?: boolean
  /** Enable the configurable pane hotkeys (split right/down, close pane) in handleKey. */
  paneHotkeys?: boolean
}

export interface AppController {
  historyRequest(tab: TabState, request: HostHistoryRequest): Promise<HostHistoryReply>
  catalogLoading: Ref<boolean>
  /** Saved runtime Host metadata (local protected-store snapshot only —
   *  never a session target and no credential material). */
  runtimeProfiles: Ref<RuntimeHostDto[]>
  /** Safe fixed-mapping text for a failed local profile read; '' when ok. */
  runtimeProfilesError: Ref<string>
  instanceDesktops: Ref<InstanceDesktop[]>
  instanceDesktopsError: Ref<string>
  loadInstanceDesktops(): Promise<void>
  selectInstance(tab: TabState, desktop: InstanceDesktop): Promise<void>
  clearInstance(tab: TabState): Promise<void>
  tabs: TabState[]
  activeKey: Ref<string | null>
  activeTab: ComputedRef<TabState | null>
  uiNotice: Ref<string>
  /** Monotonic signal: a retired runtime tab closed and the profile list
   *  was refreshed; the mobile shell surfaces the target selection list.
   *  Increment-only — works even when other tabs stay open; no payload
   *  is ever attached. */
  selectionRequest: Ref<number>
  /** Incremented after a Shell is created or explicitly deleted. */
  sessionsRevision: Ref<number>
  loadCatalog(): Promise<void>
  refreshTargets(): Promise<void>
  openTarget(host: HostUiState, target: TargetDto, withDesktop: boolean): void
  attachTerminal(tab: TabState, handle: TerminalHandle): void
  /**
   * Split `leafId` toward `direction`, adding a new independent connection
   * pane. A fresh session is created on the same runtime Host profile the
   * split pane is connected to (the sidebar "+" equivalent); resolves false
   * and leaves the tree untouched when creation is unavailable or fails.
   */
  splitPane(tab: TabState, leafId: string, direction: SplitDirection): Promise<boolean>
  isPaneKey(e: KeyboardEvent): boolean
  /** Explicitly delete the selected remote Shell, then close its pane. */
  closePaneLeaf(tab: TabState, leafId: string): Promise<void>
  /** Remove every non-primary pane leaf (child connections close). */
  unsplitPanes(tab: TabState): void
  /** Point keyboard focus at a pane leaf (null clears the focused leaf). */
  focusPane(tab: TabState, leafId: string | null): void
  /** Cycle conn-pane focus; delta +1/-1 in display order. */
  cyclePaneFocus(tab: TabState, delta: number): void
  /** Conn TabState that should receive paste/clipboard input for this tab. */
  focusedConnTab(tab: TabState): TabState
  reopenTab(tab: TabState, withDesktop: boolean): void
  closeTab(tab: TabState): Promise<void>
  detachTab(tab: TabState): Promise<void>
  setHidden(hidden: boolean): Promise<void>
  setActive(key: string): void
  stepTab(delta: number): void
  isSwitchKey(e: KeyboardEvent): boolean
  handleKey(e: KeyboardEvent): boolean
  sendInput(tab: TabState, text: string): void
  requestResize(tab: TabState, rows: number, cols: number): void
  setDesktop(tab: TabState, enabled: boolean): Promise<void>
  setPreviewVisible(tab: TabState, visible: boolean): void
  manualCapture(): void
  dispose(): void
}
