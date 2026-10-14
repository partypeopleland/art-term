<script setup lang="ts">
// Mobile shell: runtime connection list home (agents/computers from the
// saved runtime Host profiles) + narrow-screen multi-tab sessions and
// shared runtime Host settings. Tabs/poll/preview rules live in the
// shared createAppController.
import { computed, nextTick, onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import { NativeHistoryKey } from '../terminal/native'
import { createNativeHistoryReader } from './history'
import { createMobileInput, type SpecialKey } from './input'
import {
  createAppController,
  type AppController,
  type ControllerOptions,
  type HostUiState,
  type TabState
} from '../state/controller'
import {
  TerminalFactoryKey,
  createMobileXtermTerminal,
  type TerminalFactory,
  type TerminalHandle
} from '../terminal/handle'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../transport/types'
import { createRuntimeEntry, runtimeConnectedIds } from '../state/runtime-entry'
import TabBar from '../components/TabBar.vue'
import TerminalPane from '../components/TerminalPane.vue'
import PreviewPanel from '../components/PreviewPanel.vue'
import CategoryList from '../components/CategoryList.vue'
import RuntimeHosts from '../components/RuntimeHosts.vue'
import RuntimeConnectionDialog from '../components/RuntimeConnectionDialog.vue'
import VoiceButton from './VoiceButton.vue'
import { createTauriVoicePlugin, createVoiceController, type VoiceTarget } from './voice'

const props = defineProps<{
  transport: Transport
  terminalFactory?: TerminalFactory
  controllerOptions?: ControllerOptions
}>()

const controller: AppController = createAppController(
  props.transport,
  { ...props.controllerOptions, backgroundConnection: 'keep' }
)
// MobileApp is only loaded after gui_platform identifies Android in main.ts.
const voice = createVoiceController(createTauriVoicePlugin())
const input = createMobileInput()
provide(NativeHistoryKey, createNativeHistoryReader)
defineExpose({ controller })

// The injected factory wraps every real terminal with the IME/latch adapter,
// so shared TerminalPane + shared controller see only clean input.
const baseFactory = props.terminalFactory ?? createMobileXtermTerminal
provide(TerminalFactoryKey, (el: HTMLElement): TerminalHandle =>
  input.wrap(el, baseFactory(el))
)

const rootEl = ref<HTMLElement | null>(null)
const composerOpen = ref(false)
const composerText = ref('')
const phase = ref<'list' | 'session'>('list')
const runtimeHostsOpen = ref(false)
const transferPicker = ref(false)
const hostsView = ref<'list' | 'create'>('create')
// Every transport in the runtime-only product implements the full
// runtime Host surface; the flag only gates rendering.
const runtimeAvailable = computed(() => true)

function openRuntimeHosts(view: 'list' | 'create' = 'create'): void {
  hostsView.value = view
  runtimeEntry.cancel()
  composerOpen.value = false
  composerText.value = ''
  runtimeHostsOpen.value = true
}

function openRuntimeSession(host: HostDto, target: TargetDto): void {
  runtimeHostsOpen.value = false
  pickTarget({ dto: host, expanded: true }, target, false)
}

const runtimeDialogProfile = ref<RuntimeHostDto | null>(null)
const runtimeDialogSeed = ref<HostDto | null>(null)

function openRuntimeEntry(profile: RuntimeHostDto, seed: HostDto | null = null): void {
  composerOpen.value = false
  composerText.value = ''
  // Frozen snapshot: a later catalog reload cannot swap the id/revision the
  // dialog lists or creates sessions against.
  runtimeDialogSeed.value = seed
  runtimeDialogProfile.value = { ...profile }
}

function openRuntimeTarget(host: HostDto, target: TargetDto): void {
  runtimeDialogProfile.value = null
  runtimeDialogSeed.value = null
  pickTarget({ dto: host, expanded: true }, target, false)
}

// Green dot input: local live runtime tabs only — never a remote probe.
const runtimeConnected = computed(() => runtimeConnectedIds(controller.tabs))

// Shared entry behaviour, same contract as the desktop shell: primary click
// reuses a live same-host tab (landing on the session phase), else one
// bounded list with a single-running-ready-Shell fast path; the right
// Shell button always opens the chooser.
const runtimeEntry = createRuntimeEntry(props.transport, controller, {
  activateTab: (tab) => {
    controller.setActive(tab.key)
    phase.value = 'session'
  },
  openChooser: (profile, seed) => openRuntimeEntry(profile, seed),
  openTarget: openRuntimeTarget,
  showError: (message) => {
    controller.uiNotice.value = message
  }
})

const KEYBAR: { id: SpecialKey; label: string }[] = [
  { id: 'esc', label: 'Esc' },
  { id: 'tab', label: 'Tab' },
  { id: 'left', label: '←' },
  { id: 'up', label: '↑' },
  { id: 'down', label: '↓' },
  { id: 'right', label: '→' },
  { id: 'shiftleft', label: '⇧←' },
  { id: 'ctrlc', label: 'Ctrl+C' },
  { id: 'detach', label: 'Detach' }
]

const activeTab = controller.activeTab
const voiceTarget = computed<VoiceTarget | null>(() => {
  const tab = activeTab.value
  if (!tab || phase.value !== 'session') return null
  const generation = tab.connectionGen
  return {
    tabId: tab.key,
    generation,
    isValid: () => phase.value === 'session' &&
      controller.tabs.includes(tab) &&
      activeTab.value === tab &&
      !tab.closed &&
      tab.status === 'live' &&
      tab.connectionGen === generation,
    bracketedPasteMode: () => tab.term?.bracketedPasteMode?.() ?? false,
    send: (text) => controller.sendInput(tab, text)
  }
})

watch(
  () => [phase.value, activeTab.value?.key, activeTab.value?.connectionGen, activeTab.value?.status] as const,
  (next, previous) => {
    if (next.some((value, index) => value !== previous[index])) voice.cancel('target')
  }
)

const statusText = computed(() => {
  const t = activeTab.value
  if (!t) return '未連線'
  switch (t.status) {
    case 'connecting':
      return '連線中…'
    case 'live':
      return '已連線'
    case 'exited':
      return '已結束'
    default:
      return '已中斷'
  }
})

// A retired runtime tab closed and reloaded the profile list — always
// land on the selection list (works even when other tabs stay open).
watch(controller.selectionRequest, () => {
  composerOpen.value = false
  composerText.value = ''
  phase.value = 'list'
})

function tapKey(id: SpecialKey): void {
  const tab = activeTab.value
  if (!tab) return
  if (id === 'detach') {
    // Local detach only — the remote tmux session survives on the host and
    // the tab keeps its exact target for a one-tap reconnect.
    void controller.detachTab(tab)
    return
  }
  controller.sendInput(tab, input.bytesFor(id))
  tab.term?.focus()
}

function tapCtrl(): void {
  input.toggleCtrl()
  activeTab.value?.term?.focus()
}

function composerSend(withEnter: boolean): void {
  const tab = activeTab.value
  const text = composerText.value + (withEnter ? '\r' : '')
  composerOpen.value = false
  composerText.value = ''
  if (tab && text) controller.sendInput(tab, text)
  tab?.term?.focus()
}

function pickTarget(host: HostUiState, target: TargetDto, withDesktop: boolean): void {
  runtimeEntry.cancel()
  controller.openTarget(host, target, withDesktop)
  phase.value = 'session'
}

async function toggleDesktop(tab: TabState): Promise<void> {
  const enabling = !tab.desktopEnabled
  await controller.setDesktop(tab, enabling)
  // Re-enabling restores a hidden image; the panel corner toggle owns
  // hide/show from then on (the shared desktop is never affected).
  if (enabling && tab.desktopEnabled && !tab.previewVisible) {
    controller.setPreviewVisible(tab, true)
  }
}

function backToList(): void {
  phase.value = 'list'
  void controller.loadCatalog()
}

function onGlobalKey(e: KeyboardEvent): void {
  // Capture phase: Alt+[/Alt+] and Ctrl+PgUp/PgDn switch tabs before xterm
  // can turn them into remote bytes.
  controller.handleKey(e)
}

function syncViewport(): void {
  const vv = window.visualViewport
  if (vv && rootEl.value) rootEl.value.style.height = `${vv.height}px`
}

function onViewport(): void {
  syncViewport()
  const tab = activeTab.value
  if (tab?.term) {
    tab.term.fit()
    controller.requestResize(tab, tab.term.rows, tab.term.cols)
  }
}

function onVisibility(): void {
  if (document.hidden) {
    // Password and queued input never survive backgrounding.
    composerOpen.value = false
    composerText.value = ''
    if (!transferPicker.value) runtimeHostsOpen.value = false
    runtimeDialogProfile.value = null
    runtimeDialogSeed.value = null
    input.setSuspended(true)
    voice.cancel('background')
  }
  // Keep runtime sessions during App switches; hidden preview capture
  // pauses and unsent input is discarded. Explicit Detach still closes
  // locally.
  if (document.hidden) void controller.setHidden(true)
  else {
    input.setSuspended(false)
    syncViewport()
    void nextTick(() => { onViewport(); void controller.setHidden(false) })
  }
}

onMounted(() => {
  syncViewport()
  void voice.initialize()
  if (document.hidden) onVisibility()
  window.addEventListener('keydown', onGlobalKey, true)
  document.addEventListener('visibilitychange', onVisibility)
  window.visualViewport?.addEventListener('resize', onViewport)
  window.addEventListener('orientationchange', onViewport)
  window.addEventListener('resize', onViewport)
  // Home is the runtime connection list itself — load it directly.
  void controller.loadCatalog()
})

onBeforeUnmount(() => {
  input.setSuspended(true)
  voice.dispose()
  window.removeEventListener('keydown', onGlobalKey, true)
  document.removeEventListener('visibilitychange', onVisibility)
  window.visualViewport?.removeEventListener('resize', onViewport)
  window.removeEventListener('orientationchange', onViewport)
  window.removeEventListener('resize', onViewport)
  // App teardown: local detach for every live session (remote tmux/desktop
  // survives on the host) plus terminal disposal.
  for (const tab of [...controller.tabs]) void controller.detachTab(tab)
  for (const tab of controller.tabs) {
    tab.term?.dispose()
    tab.term = null
  }
  runtimeEntry.dispose()
  controller.dispose()
})
</script>

<template>
  <div ref="rootEl" class="m-app">
    <RuntimeHosts v-if="runtimeHostsOpen" :transport="transport" :initial-view="hostsView"
      @picker="transferPicker = $event" @close="runtimeHostsOpen = false" @changed="controller.loadCatalog()" @open="openRuntimeSession" />
    <RuntimeConnectionDialog v-if="runtimeDialogProfile" :profile="runtimeDialogProfile" :transport="transport"
      :initial-group="runtimeDialogSeed"
      @open="openRuntimeTarget" @close="runtimeDialogProfile = null; runtimeDialogSeed = null" />
    <!-- ── 分類連線列表（runtime 連線首頁）── -->
    <div v-if="phase === 'list'" class="page list-page">
      <header class="m-head">
        <span class="m-title">art-term</span>
        <span class="m-sub">連線</span>
        <span class="flex" />
        <button
          v-if="controller.tabs.length"
          class="mbtn slim"
          data-testid="open-tabs"
          @click="phase = 'session'"
        >分頁（{{ controller.tabs.length }}）</button>
      </header>

      <section class="card" data-testid="catalog-card">
        <div v-if="controller.uiNotice.value" class="warn" data-testid="list-notice">
          {{ controller.uiNotice.value }}
        </div>
        <CategoryList
          :catalog-loading="controller.catalogLoading.value"
          :runtime-profiles="controller.runtimeProfiles.value"
          :runtime-profiles-error="controller.runtimeProfilesError.value"
          :runtime-pending="runtimeEntry.pendingId.value"
          :runtime-connected="runtimeConnected"
          @runtime-open="runtimeEntry.primary"
          @runtime-shell="runtimeEntry.shell"
          @create="openRuntimeHosts('create')"
          @manage="openRuntimeHosts('list')"
          @refresh="controller.loadCatalog()"
        />
      </section>
    </div>

    <!-- ── 多連線終端頁 ── -->
    <!-- v-show, not v-if: TerminalPane instances must live for the whole tab
         lifetime so hidden-tab buffers keep draining and scrollback survives
         list↔session navigation. -->
    <div v-show="phase === 'session'" class="page term-page">
      <header class="t-head">
        <button class="mbtn slim" data-testid="to-list" @click="backToList">連線</button>
        <button v-if="runtimeAvailable" class="mbtn slim" data-testid="runtime-hosts-session-open" @click="openRuntimeHosts()">新增 openab-pty 連線</button>
        <span class="chip" :class="activeTab?.status ?? 'idle'" data-testid="link-status">
          {{ statusText }}
        </span>
        <span class="flex" />
        <button v-if="activeTab?.codingCli" class="mbtn slim" data-testid="launch-coding-cli"
          :disabled="activeTab.status !== 'live'" @click="controller.launchCodingCli(activeTab)">{{ activeTab.codingCli.name }}</button>
        <button
          v-if="activeTab && activeTab.status === 'live' && activeTab.desktopAvailable"
          class="mbtn slim"
          data-testid="desktop-toggle"
          :disabled="activeTab.desktopBusy"
          :aria-pressed="activeTab.desktopEnabled"
          :aria-label="activeTab.desktopEnabled ? '停用桌面預覽' : '啟用桌面預覽'"
          @click="toggleDesktop(activeTab)"
        >桌面</button>
      </header>

      <TabBar
        :tabs="controller.tabs"
        :active-key="controller.activeKey.value"
        @select="(t) => { runtimeEntry.cancel(); controller.setActive(t.key) }"
        @close="(t) => void controller.closeTab(t)"
      />

      <div class="t-area">
        <TerminalPane
          v-for="tab in controller.tabs"
          :key="tab.key"
          :tab="tab"
          :active="tab.key === controller.activeKey.value"
          :controller="controller"
          :reader-visible="phase === 'session' && !composerOpen && !runtimeHostsOpen && !runtimeDialogProfile"
        />
        <div v-if="!controller.tabs.length" class="cover" data-testid="no-tabs">
          <div class="done">
            <div class="hint">尚未開啟連線；回列表選擇一個連線。</div>
            <button class="mbtn primary" data-testid="no-tabs-targets" @click="backToList">
              選擇連線
            </button>
          </div>
        </div>
      </div>

      <!-- Mount during enablement and on failure too, so loading and errors
           stay visible even when the desktop has not become enabled yet.
           The corner toggle gates image capture only. -->
      <div v-if="activeTab && (activeTab.desktopEnabled || activeTab.desktopBusy || activeTab.desktopError)" class="sheet-host" data-testid="preview-sheet">
        <PreviewPanel :tab="activeTab" :controller="controller" />
      </div>

      <div class="keybar" data-testid="keybar">
        <button
          class="kbtn"
          :class="{ on: input.ctrlLatch.value }"
          data-testid="k-ctrl"
          @click="tapCtrl"
        >Ctrl</button>
        <button
          v-for="k in KEYBAR"
          :key="k.id"
          class="kbtn"
          :data-testid="`k-${k.id}`"
          @click="tapKey(k.id)"
        >{{ k.label }}</button>
        <button class="kbtn" data-testid="k-composer" @click="composerOpen = true">輸入</button>
        <button class="kbtn" data-testid="k-keyboard" @click="activeTab?.term?.focus()">鍵盤</button>
      </div>

      <VoiceButton
        v-if="voice.state.pluginPresent && phase === 'session' && activeTab && voiceTarget"
        :voice="voice"
        :target="voiceTarget"
      />

      <div class="t-foot">
        <span v-if="activeTab?.remoteSession" class="mono" data-testid="remote-session">
          {{ activeTab.remoteSession }}
        </span>
        <span v-if="activeTab?.notice" class="warn">{{ activeTab.notice }}</span>
        <span v-if="controller.uiNotice.value" class="warn">{{ controller.uiNotice.value }}</span>
      </div>

      <div v-if="composerOpen" class="modal" data-testid="composer">
        <div class="sheet">
          <div class="card-title">多行輸入／貼上</div>
          <textarea
            v-model="composerText"
            data-testid="composer-text"
            rows="5"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            placeholder="可貼上多行內容；送出後以原始位元組寫入遠端"
          />
          <div class="actions">
            <button class="mbtn primary" data-testid="composer-send" @click="composerSend(false)">
              送出
            </button>
            <button class="mbtn" data-testid="composer-send-enter" @click="composerSend(true)">
              送出＋Enter
            </button>
            <button class="mbtn" data-testid="composer-cancel" @click="composerOpen = false">
              取消
            </button>
          </div>
        </div>
      </div>
    </div>

  </div>
</template>

<style scoped>
.m-app {
  position: relative;
  --bg: #16171b;
  --bg-panel: #1d1f24;
  --bg-raised: #26282e;
  --bg-hover: #2e3038;
  --border: #33353d;
  --text: #d7d7dc;
  --text-dim: #9a9ca6;
  --accent: #5b8def;
  --ok: #6a9955;
  --warn: #d7ba7d;
  --err: #d16969;
  height: 100%;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
  font-family: 'Segoe UI', 'Microsoft JhengHei', system-ui, sans-serif;
  font-size: 14px;
}

.page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
}

.list-page {
  overflow-y: auto;
  padding: 10px;
  gap: 10px;
}

.m-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 4px 2px;
}

.m-title {
  font-size: 18px;
  font-weight: 700;
}

.m-sub {
  color: var(--text-dim);
}

.card {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.card :deep(.tree) {
  padding: 0;
}

.card :deep(.tree-title) {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.card-title {
  font-size: 12px;
  letter-spacing: 0.08em;
  color: var(--text-dim);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 3px;
  flex: 1;
}

.field span {
  font-size: 12px;
  color: var(--text-dim);
}

.field input {
  background: var(--bg-raised);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  padding: 8px 10px;
  font-size: 15px;
}

.field input:focus {
  outline: none;
  border-color: var(--accent);
}

.field input:disabled {
  opacity: 0.6;
}

.hint {
  color: var(--text-dim);
  font-size: 12px;
  line-height: 1.5;
}

.mono {
  font-family: Consolas, 'Cascadia Mono', monospace;
  font-size: 12px;
}

.err {
  color: var(--err);
  word-break: break-all;
}

.warn {
  color: var(--warn);
}

.actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.mbtn {
  background: var(--bg-raised);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 14px;
  font-size: 14px;
  cursor: pointer;
}

.mbtn:active {
  background: var(--bg-hover);
}

.mbtn:disabled {
  opacity: 0.5;
}

.mbtn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #0e1116;
  font-weight: 600;
}

.mbtn.slim {
  padding: 4px 10px;
}

/* ── terminal page ── */
.term-page {
  min-height: 0;
}

.t-head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--border);
  flex: none;
}

.chip {
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 1px 8px;
  color: var(--text-dim);
  white-space: nowrap;
  flex: none;
}

.chip.live {
  color: var(--ok);
  border-color: var(--ok);
}

.chip.connecting {
  color: var(--warn);
  border-color: var(--warn);
}

.chip.error,
.chip.exited {
  color: var(--err);
  border-color: var(--err);
}

.flex {
  flex: 1;
}

.t-area {
  flex: 1;
  position: relative;
  min-height: 0;
  overflow: hidden;
  background: var(--bg);
}

.t-area :deep(.pane) {
  position: absolute;
  inset: 0;
}

.t-area :deep(.term-host) {
  position: absolute;
  inset: 0;
  padding: 4px 6px;
  overflow: hidden;
}

.t-area :deep(.term-host .xterm) {
  height: 100%;
}

.t-area :deep(.overlay) {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(22, 23, 27, 0.82);
}

.t-area :deep(.done) {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 0 20px;
  text-align: center;
}

.t-area :deep(.btn) {
  background: var(--bg-raised);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 14px;
}

.t-area :deep(.error) {
  color: var(--err);
}

.t-area :deep(.muted) {
  color: var(--text-dim);
}

.cover {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.done {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 0 20px;
  text-align: center;
}

/* narrow-screen TabBar reuse: horizontal scroll, compact tabs */
.term-page :deep(.tabbar) {
  display: flex;
  align-items: stretch;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--border);
  overflow-x: auto;
  min-height: 34px;
  flex: none;
}

.term-page :deep(.tab) {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  border-right: 1px solid var(--border);
  white-space: nowrap;
  max-width: 150px;
  font-size: 13px;
}

.term-page :deep(.tab.active) {
  background: var(--bg);
  box-shadow: inset 0 2px 0 var(--accent);
}

.term-page :deep(.tab .name) {
  overflow: hidden;
  text-overflow: ellipsis;
}

.term-page :deep(.tab .glyph.live) {
  color: var(--ok);
}

.term-page :deep(.tab .glyph.connecting) {
  color: var(--warn);
}

.term-page :deep(.tab .close) {
  background: none;
  border: none;
  color: var(--text-dim);
  font-size: 16px;
  padding: 2px 4px;
}

.term-page :deep(.tab .hint) {
  padding: 8px 10px;
  font-size: 12px;
}

.keybar {
  display: flex;
  gap: 4px;
  padding: 4px 6px;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
  overflow-x: auto;
  flex: none;
}

.kbtn {
  background: var(--bg-raised);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 7px 12px;
  font-size: 13px;
  white-space: nowrap;
  cursor: pointer;
  flex: none;
}

.kbtn:active {
  background: var(--bg-hover);
}

.kbtn.on {
  background: var(--accent);
  border-color: var(--accent);
  color: #0e1116;
  font-weight: 700;
}

.t-foot {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 3px 10px;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-dim);
  min-height: 20px;
  flex: none;
}

.modal {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.55);
  display: flex;
  align-items: flex-end;
  z-index: 10;
}

.sheet {
  width: 100%;
  max-height: 70%;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
  border-radius: 12px 12px 0 0;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.sheet-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.sheet-body {
  overflow-y: auto;
  min-height: 0;
}

.sheet-body :deep(.tree) {
  padding: 4px 0;
}

.sheet-body :deep(.tree-title) {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.sheet-body :deep(.host-row),
.sheet-body :deep(.target) {
  padding-top: 8px;
  padding-bottom: 8px;
}

.sheet textarea {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  padding: 8px 10px;
  font-family: Consolas, 'Cascadia Mono', monospace;
  font-size: 14px;
  resize: none;
}

.sheet textarea:focus {
  outline: none;
  border-color: var(--accent);
}

.sheet-host {
  position: relative;
  flex: none;
  max-height: 45%;
  min-height: 0;
  overflow-y: auto;
  border-top: 1px solid var(--border);
}

.sheet-host :deep(.preview) {
  border-top: none;
  max-height: none;
  overflow: visible;
}

.m-app :deep(.runtime-host-backdrop),
.m-app :deep(.runtime-conn-backdrop) {
  position: absolute;
  padding: 8px;
  align-items: flex-end;
}
.m-app :deep(.runtime-hosts),
.m-app :deep(.runtime-conn) {
  width: 100%;
  max-height: 100%;
  box-sizing: border-box;
  padding: 12px;
  overscroll-behavior: contain;
}
.m-app :deep(.runtime-hosts input),
.m-app :deep(.runtime-conn input) { font-size: 16px; min-height: 44px; box-sizing: border-box; }
.m-app :deep(.runtime-hosts button),
.m-app :deep(.runtime-conn button) { min-height: 44px; touch-action: manipulation; }
.m-app :deep(.runtime .mini) { min-height: 44px; min-width: 44px; touch-action: manipulation; }

.sheet-host :deep(.shot) {
  max-height: 30vh;
  object-fit: contain;
}
</style>
