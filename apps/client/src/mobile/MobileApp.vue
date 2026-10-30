<script setup lang="ts">
// Mobile shell: runtime connection list home (agents/computers from the
// saved runtime Host profiles) + narrow-screen multi-tab sessions and
// shared runtime Host settings. Tabs/poll/preview rules live in the
// shared createAppController.
import { computed, inject, nextTick, onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import { NativeHistoryKey, NativeKeyboardKey, NativeSwipeKey, type NativeHistoryReader } from '../shared/terminal/native'
import { createNativeHistoryReader, listenKeyboardVisibility } from './history'
import { createMobileInput, type SpecialKey } from './input'
import {
  createAppController,
  type AppController,
  type ControllerOptions,
  type HostUiState,
  type TabState
} from '../shared/state/controller'
import {
  TerminalFactoryKey,
  createMobileXtermTerminal,
  type TerminalFactory,
  type TerminalHandle
} from '../shared/terminal/handle'
import type { InstanceDesktop, HostDto, RuntimeHostDto, TargetDto, Transport } from '../shared/transport/types'
import { createRuntimeEntry, runtimeConnectedIds } from '../shared/state/runtime-entry'
import TabBar from '../shared/components/TabBar.vue'
import TerminalPane from '../shared/components/TerminalPane.vue'
import PreviewPanel from '../shared/components/PreviewPanel.vue'
import PreviewZoom from '../shared/components/PreviewZoom.vue'
import ConnectionDashboard from '../shared/components/ConnectionDashboard.vue'
import ThemeSettings from '../shared/components/ThemeSettings.vue'
import ScreenPicker from '../shared/components/ScreenPicker.vue'
import AppHomepage from '../shared/components/AppHomepage.vue'
import { selectedTheme, terminalTheme } from '../shared/theme/theme'
import '../shared/theme/themes.css'
import { useWorkspacePager, type Workspace } from './pager'
import RuntimeHosts from '../shared/components/RuntimeHosts.vue'
import RuntimeConnectionDialog from '../shared/components/RuntimeConnectionDialog.vue'
import VoiceButton from './VoiceButton.vue'
import { createTauriVoicePlugin, createVoiceController, type VoiceTarget, type VoicePlugin } from './voice'

const props = defineProps<{
  transport: Transport
  terminalFactory?: TerminalFactory
  controllerOptions?: ControllerOptions
  /** null selects the browser xterm reader for the isolated UI preview. */
  nativeHistoryFactory?: (() => NativeHistoryReader) | null
  /** Optional browser demo; production uses the Android voice plugin. */
  voicePlugin?: VoicePlugin
}>()

const controller: AppController = createAppController(
  props.transport,
  { ...props.controllerOptions, backgroundConnection: 'keep', closeFinishedTabs: true }
)
// Production loads this after identifying Android; browser preview aliases native IPC.
const voice = createVoiceController(props.voicePlugin ?? createTauriVoicePlugin(), props.voicePlugin ? { isTauri: () => true } : {})
const input = createMobileInput()
if (props.nativeHistoryFactory !== null) {
  provide(NativeHistoryKey, props.nativeHistoryFactory ?? createNativeHistoryReader)
}
defineExpose({ controller })

// The injected factory wraps every real terminal with the IME/latch adapter,
// so shared TerminalPane + shared controller see only clean input.
const baseFactory = props.terminalFactory ?? createMobileXtermTerminal
provide(TerminalFactoryKey, (el: HTMLElement): TerminalHandle => {
  const terminal = input.wrap(el, baseFactory(el))
  terminal.setTheme?.(terminalTheme(selectedTheme.value))
  return terminal
})
watch(selectedTheme, theme => {
  for (const tab of controller.tabs) tab.term?.setTheme?.(terminalTheme(theme))
})

const rootEl = ref<HTMLElement | null>(null)
const settingsOpen = ref(false)
const screenPickerOpen = ref(false)
const zoomOpen = ref(false)
const aboutOpen = ref(false)
function showHomepage(): void {
  runtimeEntry.cancel()
  voice.cancel('target')
  settingsOpen.value = false
  composerOpen.value = false
  aboutOpen.value = true
}
const voiceLightboxOpen = ref(false)
const composerOpen = ref(false)
const composerText = ref('')
const phase = ref<Workspace>('list')
const zoomVisible = computed(() => {
  const tab = controller.activeTab.value
  return zoomOpen.value && phase.value === 'session' && !!tab && tab.previewVisible && (tab.desktopEnabled || !!tab.shot)
})
const pager = useWorkspacePager(rootEl, phase, activateWorkspace, () => !zoomVisible.value && !screenPickerOpen.value && !aboutOpen.value && !settingsOpen.value && !runtimeHostsOpen.value && !runtimeDialogProfile.value && !composerOpen.value)
const { dragging: swipeDragging, animating: swipeAnimating, nativeDragging: nativeSwipeActive, visible: swipeVisible, indicatorStyle: workspaceIndicatorStyle, transform: pageTransform, begin: beginSwipe, move: moveSwipe, end: finishSwipe, guardClick: guardSwipeClick } = pager
provide(NativeSwipeKey, pager.native)
function activateWorkspace(page: Workspace): void {
  if (page === 'list') backToList()
  else switchWorkspace(page)
}
function slideTo(workspace: Workspace): void {
  aboutOpen.value = false
  settingsOpen.value = false
  pager.slideTo(workspace)
}

const runtimeHostsOpen = ref(false)
const transferPicker = ref(false)
const hostsView = ref<'list' | 'create'>('create')
const hostsCategory = ref<'host' | 'agent' | undefined>()
// Every transport in the runtime-only product implements the full
// runtime Host surface; the flag only gates rendering.
const runtimeAvailable = computed(() => true)

function openRuntimeHosts(view: 'list' | 'create' = 'create', category?: 'host' | 'agent'): void {
  hostsView.value = view
  hostsCategory.value = category
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
  { id: 'tab', label: 'Tab' },
  { id: 'esc', label: 'Esc' },
  { id: 'shiftleft', label: '⇧←' },
  { id: 'up', label: '↑' },
  { id: 'down', label: '↓' },
  { id: 'left', label: '←' },
  { id: 'right', label: '→' },
  { id: 'enter', label: 'ENTER' },
  { id: 'ctrlc', label: 'CTRL+C' }
]

const activeTab = controller.activeTab
const keyboardVisible = ref<boolean | null>(null)
const subscribeKeyboard = inject(NativeKeyboardKey, listenKeyboardVisibility)
let removeKeyboard: (() => Promise<void>) | undefined
let disposed = false
function keepKeyboardState(): void {
  if (keyboardVisible.value === false) activeTab.value?.term?.suppressSoftKeyboard?.()
}
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

// Show the complete bottom bar in the browser even before a demo tab is attached.
const displayedVoiceTarget = computed<VoiceTarget | null>(() => voiceTarget.value ?? (props.voicePlugin ? {
  tabId: 'preview-layout', generation: 0,
  isValid: () => phase.value === 'session' && !document.hidden,
  bracketedPasteMode: () => false,
  send: () => {}
} : null))
const displayedRemoteSession = computed(() => activeTab.value?.remoteSession || (props.voicePlugin ? 'openab-pty:preview（版面示範）' : ''))

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
}

function tapCtrl(): void {
  input.toggleCtrl()
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

function screensChanged(): void { void controller.loadInstanceDesktops(); void controller.loadCatalog() }
function openScreenPicker(): void { screenPickerOpen.value = true; void controller.loadInstanceDesktops() }
async function selectScreen(desktop: InstanceDesktop): Promise<void> {
  const tab = activeTab.value
  if (!tab) return
  await controller.selectInstance(tab, desktop)
  if (tab.instanceDesktop?.id === desktop.id) screenPickerOpen.value = false
}
async function clearScreen(): Promise<void> {
  if (activeTab.value) await controller.clearInstance(activeTab.value)
  screenPickerOpen.value = false
}
watch(() => activeTab.value?.key, () => { screenPickerOpen.value = false })

async function toggleDesktop(tab: TabState): Promise<void> {
  const enabling = !tab.desktopEnabled
  await controller.setDesktop(tab, enabling)
  // Re-enabling restores a hidden image; the panel corner toggle owns
  // hide/show from then on (the shared desktop is never affected).
  if (enabling && tab.desktopEnabled && !tab.previewVisible) {
    controller.setPreviewVisible(tab, true)
  }
}

function switchWorkspace(workspace: Workspace, focus = false): void {
  runtimeEntry.cancel()
  composerOpen.value = false
  phase.value = workspace
  if (focus) void nextTick(() => rootEl.value?.querySelector<HTMLElement>(workspace === 'list' ? '#workspace-connections' : '#workspace-terminal')?.focus())
  if (workspace === 'session') void nextTick(onViewport)
}

function backToList(): void {
  phase.value = 'list'
  screenPickerOpen.value = false
  void controller.loadInstanceDesktops()
  void controller.loadCatalog()
}

function onGlobalKey(e: KeyboardEvent): void {
  if (aboutOpen.value || settingsOpen.value || phase.value !== 'session') return
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
    if (!transferPicker.value) settingsOpen.value = false
    pager.cancel()
    // Password and queued input never survive backgrounding.
    composerOpen.value = false
    composerText.value = ''
    if (!transferPicker.value) runtimeHostsOpen.value = false
    runtimeDialogProfile.value = null
    runtimeDialogSeed.value = null
    input.setSuspended(true)
    screenPickerOpen.value = false
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
  void subscribeKeyboard(visible => { if (!disposed) keyboardVisible.value = visible }).then(remove => {
    if (disposed) return remove()
    else removeKeyboard = remove
  }).catch(() => { /* Keyboard state is unavailable outside the native plugin. */ })
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
  void controller.loadInstanceDesktops()
})

onBeforeUnmount(() => {
  disposed = true
  if (removeKeyboard) void removeKeyboard().catch(() => {})
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
  <div ref="rootEl" class="m-app" :data-theme="selectedTheme" :class="{ 'swipe-animating': swipeAnimating, 'swipe-dragging': swipeDragging }" @pointerdown.capture="beginSwipe" @pointermove.capture="moveSwipe" @pointerup.capture="finishSwipe($event)" @pointercancel.capture="finishSwipe($event, true)" @click.capture="guardSwipeClick">
    <RuntimeHosts v-if="runtimeHostsOpen" :transport="transport" :initial-view="hostsView" :initial-category="hostsCategory" :category-filter="hostsCategory"
      @picker="transferPicker = $event" @close="runtimeHostsOpen = false" @changed="screensChanged" @open="openRuntimeSession" />
    <RuntimeConnectionDialog v-if="runtimeDialogProfile" :profile="runtimeDialogProfile" :transport="transport"
      :initial-group="runtimeDialogSeed"
      @open="openRuntimeTarget" @close="runtimeDialogProfile = null; runtimeDialogSeed = null" />
      <header class="t-head workspace-header">
        <button class="fixed-brand" :class="{ active: aboutOpen }" aria-label="軟體資訊首頁" @click="showHomepage">
        <span class="m-brand-mark" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="m5 6 6 6-6 6M13 18h6" /></svg></span>
        <div class="m-brand"><span class="m-title">art-term<span class="m-title-dot">.</span></span><span class="m-sub">CONNECTION HUB</span></div>
        </button>
        <nav class="page-text-navigation" aria-label="頁面切換">
          <span v-show="!settingsOpen && !aboutOpen" class="workspace-progress-indicator" :style="workspaceIndicatorStyle" aria-hidden="true" />
          <button class="page-text-link" :class="{ current: !settingsOpen && !aboutOpen && phase === 'list' }" :aria-current="!settingsOpen && !aboutOpen && phase === 'list' ? 'page' : undefined" data-testid="terminal-to-list" @click="slideTo('list')">Main</button>
          <button class="page-text-link" :class="{ current: !settingsOpen && !aboutOpen && phase === 'session' }" :aria-current="!settingsOpen && !aboutOpen && phase === 'session' ? 'page' : undefined" @click="slideTo('session')">Connect</button>

        </nav>

        <span class="flex" />

        <button v-if="!settingsOpen && !aboutOpen && phase === 'session' && activeTab?.status === 'live'" class="screen-trigger" data-testid="desktop-toggle" :disabled="activeTab.desktopBusy" :aria-pressed="!!activeTab.instanceDesktop" aria-label="選擇 Screen" title="選擇 Screen" @click="openScreenPicker"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="14" rx="2" /><path d="M8 21h8M12 18v3" /></svg></button>




        <button class="settings-trigger" :class="{ active: settingsOpen }" :aria-pressed="settingsOpen" aria-label="全域設定" title="全域設定" @click="aboutOpen = false; voice.cancel('target'); settingsOpen = true">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9.5 3-.6 2.3-2 .9-2.1-.7-2.3 4 1.6 1.7v2.3l-1.6 1.7 2.3 4 2.1-.7 2 .9.6 2.6h5l.6-2.6 2-.9 2.1.7 2.3-4-1.6-1.7v-2.3l1.6-1.7-2.3-4-2.1.7-2-.9L14.5 3Z" /><circle cx="12" cy="12" r="3" /></svg>
        </button>
      </header>
    <div class="workspace-viewport">
      <ScreenPicker v-if="screenPickerOpen && activeTab" :items="controller.instanceDesktops.value" :selected-id="activeTab.instanceDesktop?.id" :busy="activeTab.desktopBusy" :error="activeTab.desktopError || controller.instanceDesktopsError.value" @select="selectScreen" @clear="clearScreen" @close="screenPickerOpen = false" @settings="screenPickerOpen = false; settingsOpen = true; aboutOpen = false" />
      <div v-if="aboutOpen" class="software-home-page" role="region" aria-label="軟體資訊首頁"><AppHomepage /></div>
      <div v-if="settingsOpen" class="global-settings-page" role="region" aria-label="全域設定"><ThemeSettings embedded :transport="transport" :hosts="controller.runtimeProfiles.value" @changed="screensChanged" @picker="transferPicker = $event" /></div>
    <div v-show="!settingsOpen && !aboutOpen && (phase === 'list' || swipeVisible)" class="home-surface" :style="{ transform: pageTransform('list') }">


    <div id="connections-panel" class="page list-page" role="region" aria-label="連線">
      <section class="dashboard-region" data-testid="catalog-card" aria-label="連線工作區">
        <div v-if="controller.uiNotice.value" class="warn" data-testid="list-notice">
          {{ controller.uiNotice.value }}
        </div>
        <ConnectionDashboard
          :screens="controller.instanceDesktops.value"
          :screens-error="controller.instanceDesktopsError.value"
          :transport="transport"
          :active="!settingsOpen && !aboutOpen && phase === 'list'"
          :catalog-loading="controller.catalogLoading.value"
          :runtime-profiles="controller.runtimeProfiles.value"
          :runtime-profiles-error="controller.runtimeProfilesError.value"
          :runtime-pending="runtimeEntry.pendingId.value"
          :runtime-connected="runtimeConnected"
          @open="openRuntimeTarget"
          @create="openRuntimeHosts('create', $event)"
        />
      </section>
    </div>

    </div>

    <!-- ── 多連線終端頁 ── -->
    <!-- v-show, not v-if: TerminalPane instances must live for the whole tab
         lifetime so hidden-tab buffers keep draining and scrollback survives
         list↔session navigation. -->
    <div id="terminal-panel" v-show="!settingsOpen && !aboutOpen && (phase === 'session' || swipeVisible)" class="page term-page" :style="{ transform: pageTransform('session') }" role="region" aria-label="終端">



      <TabBar v-if="controller.tabs.length"
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
          :reader-visible="phase === 'session' && (!swipeVisible || nativeSwipeActive) && !screenPickerOpen && !zoomVisible && !voiceLightboxOpen && !composerOpen && !runtimeHostsOpen && !runtimeDialogProfile && !settingsOpen && !aboutOpen"
        />
        <div v-if="!controller.tabs.length" class="cover" data-testid="no-tabs">
          <div class="done">
            <button class="mbtn primary" data-testid="no-tabs-targets" @click="backToList">
              選擇連線
            </button>
          </div>
        </div>
      </div>

      <!-- Mount during enablement and on failure too, so loading and errors
           stay visible even when the desktop has not become enabled yet.
           The corner toggle gates image capture only. -->
      <div v-if="activeTab && (activeTab.desktopEnabled || activeTab.desktopBusy || activeTab.desktopError || activeTab.shot)" class="sheet-host" data-testid="preview-sheet">
        <PreviewPanel :tab="activeTab" :controller="controller" @enlarge="zoomOpen = true" />
      </div>

      <div class="keybar" data-testid="keybar">
        <button
          v-for="k in KEYBAR"
          :key="k.id"
          class="kbtn"
          :data-testid="`k-${k.id}`"
          @pointerdown.prevent="keepKeyboardState"
          @click="tapKey(k.id)"
        >{{ k.label }}</button>
        <button class="kbtn" data-testid="k-keyboard" @click="activeTab?.term?.focus()">鍵盤</button>
      </div>

      <VoiceButton
        v-if="(voice.state.pluginPresent || props.voicePlugin) && phase === 'session' && displayedVoiceTarget"
        :voice="voice"
        :target="displayedVoiceTarget"
        @overlay="voiceLightboxOpen = $event"
      />

      <div class="t-foot">
        <span v-if="displayedRemoteSession" class="mono" data-testid="remote-session">
          {{ displayedRemoteSession }}
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
    <PreviewZoom v-if="zoomVisible && activeTab" :tab="activeTab" :screen-key="`${activeTab.key}:${activeTab.instanceDesktop?.id || 'legacy'}`" mode="android" @close="zoomOpen = false" />
  </div>
</template>

<style scoped>
.m-app {
  position: relative;
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
  padding: 14px;
  gap: 12px;
  background: radial-gradient(ellipse at 90% 0%, var(--page-glow), transparent 48%), var(--bg);
}

.m-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 2px 2px 0;
}

.m-brand-mark { display: grid; place-items: center; width: 35px; height: 35px; border: 1px solid var(--border); border-radius: 10px; color: var(--accent); background: linear-gradient(145deg, var(--bg-raised), var(--bg-panel)); box-shadow: inset 0 1px 0 #ffffff0a; }
.m-brand-mark svg { width: 23px; height: 23px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
.m-brand { display: flex; flex-direction: column; gap: 3px; }

.m-title {
  font-size: 20px;
  font-weight: 650;
  letter-spacing: -.035em;
  line-height: 1;
  color: var(--text);
}
.m-title-dot { color: var(--accent); }

.m-sub {
  color: var(--text-dim);
  font-size: 8px;
  letter-spacing: .16em;
}

.dashboard-region { min-width: 0; }
.m-head .mbtn { min-height: 44px; border-color: var(--border); background: var(--bg-raised); font-size: 11px; }

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
  color: var(--on-accent);
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
  background: color-mix(in srgb, var(--bg) 88%, transparent);
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
  color: var(--on-accent);
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

.settings-trigger { display: grid; place-items: center; flex: none; width: 44px; height: 44px; padding: 11px; background: transparent; border: 1px solid transparent; border-radius: 10px; color: var(--text-dim); cursor: pointer; }
.settings-trigger:hover { color: var(--accent); background: var(--bg-raised); }
.settings-trigger:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
.settings-trigger svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }

.m-head { gap: 5px; }
.m-head .m-brand-mark { flex: none; }
@media (max-width: 340px) { .m-head { gap: 3px; } .m-head .m-title { font-size: 18px; } .header-action { padding-inline: 5px; } }
.header-action { min-height: 44px; min-width: 44px; border: 0; border-radius: 9px; padding: 0 9px; color: var(--text-dim); background: transparent; font: inherit; font-size: 12px; cursor: pointer; }
.header-add { color: var(--accent); background: var(--bg-raised); }
.header-action:hover { background: var(--bg-hover); color: var(--text); }
.header-action:focus-visible, .session-switch:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
.home-navigation { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.session-switch { min-height: 44px; display: inline-flex; align-items: center; gap: 9px; padding: 0 10px; border: 1px solid var(--border); border-radius: 9px; color: var(--accent); background: var(--bg-panel); font: inherit; font-size: 11px; cursor: pointer; }
.session-switch:disabled { color: var(--text-dim); opacity: .65; cursor: default; }
.session-switch svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; }
.session-count { min-width: 19px; padding: 2px 4px; border-radius: 4px; background: var(--bg-raised); text-align: center; font-family: monospace; }

.m-app { display: flex; flex-direction: column; background: radial-gradient(ellipse at 90% 0%, var(--page-glow), transparent 48%), var(--bg); }
.main-header { flex: none; margin: 12px 14px 8px; }
.workspace-navigation { display: flex; align-items: flex-end; justify-content: space-between; flex: none; margin: 0 14px; border-bottom: 1px solid var(--border); }
.workspace-tabs { display: flex; align-items: stretch; gap: 3px; }
.workspace-tab { display: inline-flex; align-items: center; justify-content: center; gap: 7px; min-height: 40px; padding: 0 14px; border: 1px solid transparent; border-bottom: 0; border-radius: 8px 8px 0 0; background: transparent; color: var(--text-dim); font: inherit; font-size: 12px; cursor: pointer; position: relative; }
.workspace-tab.active { color: var(--accent); background: var(--bg-panel); border-color: var(--border); }
.workspace-tab.active::after { position: absolute; content: ''; height: 2px; bottom: 0; left: 12px; right: 12px; background: var(--accent); border-radius: 2px; }
.workspace-tab:focus-visible { outline: 2px solid var(--accent); outline-offset: -3px; }
.workspace-navigation > .settings-trigger { width: 40px; height: 40px; padding: 11px; }
.page { flex: 1; height: auto; min-height: 0; }
.list-page { padding: 5px 14px 14px; gap: 0; background: transparent; touch-action: pan-y; }
.term-page { margin: 0 14px 10px; border: 1px solid var(--border); border-top: 0; border-radius: 0 0 10px 10px; overflow: hidden; }
.t-head { padding: 5px 8px; }

.term-page { margin: 0; border: 0; border-radius: 0; }
.t-head { flex-wrap: nowrap; gap: 5px; padding: 4px 8px; }
.t-head .mbtn { flex: none; white-space: nowrap; }
.terminal-settings { flex: none; width: 30px; height: 30px; display: grid; place-items: center; padding: 6px; border: 0; border-radius: 6px; background: transparent; color: var(--text-dim); cursor: pointer; }
.terminal-settings svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.terminal-settings:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

.home-surface { position: absolute; inset: 0; display: flex; flex-direction: column; min-height: 0; background: radial-gradient(ellipse at 90% 0%, var(--page-glow), transparent 48%), var(--bg); touch-action: pan-y; }
.term-page { position: absolute; inset: 0; background: var(--bg); touch-action: pan-y; }
.t-head { touch-action: pan-y; }
.swipe-animating .home-surface, .swipe-animating .term-page { transition: transform 200ms cubic-bezier(.2,.7,.3,1); }
.swipe-dragging .home-surface, .swipe-dragging .term-page { user-select: none; }
.swipe-dragging .home-surface { will-change: transform; }
@media (prefers-reduced-motion: reduce) { .swipe-animating .home-surface, .swipe-animating .term-page { transition: none; } }

.page-text-navigation { display: inline-flex; align-items: center; gap: 7px; flex: none; color: var(--text-dim); font-size: 11px; }
.page-text-link { border: 0; border-radius: 0; background: transparent; padding: 5px 0; color: var(--text-dim); font: inherit; cursor: pointer; }
.page-text-link.current { color: var(--accent); }
.page-text-link:hover { color: var(--text); }
.page-text-link:focus-visible { outline: 1px solid var(--accent); outline-offset: 3px; }
.main-header .page-text-navigation { gap: 4px; font-size: 10px; }

.page-text-navigation, .main-header .page-text-navigation { gap: 2px; font-size: 11px; align-items: stretch; }
.page-text-link { position: relative; min-height: 30px; padding: 7px 9px; color: var(--text-dim); border: 0; background: transparent; }
.page-text-link.current { color: var(--accent); font-weight: 600; }
.page-text-link.current::after { content: ''; position: absolute; left: 8px; right: 8px; bottom: 0; height: 2px; border-radius: 2px; background: var(--accent); }
.main-header .page-text-link { padding-inline: 6px; }
.main-header .page-text-link.current::after { left: 5px; right: 5px; }

.page-text-navigation, .main-header .page-text-navigation { position: relative; display: inline-grid; grid-template-columns: repeat(2, 1fr); gap: 0; }
.page-text-link, .main-header .page-text-link { min-width: 54px; padding-inline: 7px; text-align: center; }
.page-text-link.current::after { display: none; }
.workspace-progress-indicator { position: absolute; left: 0; bottom: 0; width: 50%; height: 2px; pointer-events: none; z-index: 1; }
.workspace-progress-indicator::after { content: ''; position: absolute; inset: 0 7px; border-radius: 2px; background: var(--accent); }
.swipe-animating .workspace-progress-indicator { transition: transform 200ms cubic-bezier(.2,.7,.3,1); }
@media (prefers-reduced-motion: reduce) { .swipe-animating .workspace-progress-indicator { transition: none; } }

.workspace-header { position: relative; z-index: 2; flex: none; min-height: 36px; padding: 3px 10px; background: var(--bg); border-bottom: 1px solid var(--border); }
.workspace-header .page-text-navigation { align-self: stretch; }
.workspace-viewport { position: relative; flex: 1; min-height: 0; overflow: hidden; }
.home-surface, .term-page { width: 100%; height: 100%; }

.workspace-header { gap: 7px; padding: 4px 10px; min-height: 44px; }
.fixed-brand { display: flex; align-items: center; gap: 6px; flex: none; }
.fixed-brand .m-brand-mark { width: 25px; height: 25px; border-radius: 7px; }
.fixed-brand .m-brand-mark svg { width: 17px; height: 17px; }
.fixed-brand .m-title { font-size: 15px; }
.fixed-brand .m-sub { display: none; }
.workspace-header .settings-trigger { width: 30px; height: 30px; padding: 7px; }
.workspace-header .settings-trigger svg { width: 17px; height: 17px; }
.workspace-header .page-text-link { min-width: 48px; padding-inline: 5px; font-size: 10px; }
.list-page { padding-top: 8px; }

.workspace-header .page-text-navigation { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); flex: 1; margin-left: auto; max-width: 260px; }
.workspace-header .page-text-link { min-width: 0; width: 100%; padding-inline: 4px; font-size: 11px; }
.workspace-progress-indicator { width: 25%; }
.settings-page-tab { display: grid; place-items: center; }
.settings-page-tab svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.workspace-page { position: absolute; inset: 0; overflow-y: auto; background: var(--bg); touch-action: pan-y; }
.swipe-animating .workspace-page { transition: transform 200ms cubic-bezier(.2,.7,.3,1); }
.swipe-dragging .workspace-page { user-select: none; }
@media (prefers-reduced-motion: reduce) { .swipe-animating .workspace-page { transition: none; } }

.workspace-header { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 12px; width: 100%; padding: 4px 10px; }
.workspace-header .fixed-brand { min-width: 0; }
.workspace-header .page-text-navigation { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); width: 100%; min-width: 0; max-width: none; margin: 0; gap: 0; }
.workspace-header .page-text-link { display: grid; place-items: center; min-width: 0; width: 100%; height: 32px; margin: 0; padding: 4px 2px; font-size: 11px; }
.workspace-header .workspace-progress-indicator { width: 25%; }
.terminal-tools { display: flex; justify-content: flex-end; gap: 5px; padding: 3px 8px; background: var(--bg-panel); }

.workspace-header { display: flex; gap: 7px; padding: 4px 10px; min-height: 44px; }
.workspace-header .page-text-navigation { display: inline-grid; grid-template-columns: repeat(2, 1fr); flex: none; width: auto; min-width: 0; max-width: none; margin: 0; gap: 0; }
.workspace-header .page-text-link { min-width: 48px; width: auto; height: 30px; padding: 5px; font-size: 10px; }
.workspace-header .workspace-progress-indicator { width: 50%; }

.global-settings-page { position: absolute; inset: 0; overflow-y: auto; background: var(--bg); }
.workspace-header .settings-trigger { position: relative; }
.workspace-header .settings-trigger.active { color: var(--accent); }
.workspace-header .settings-trigger.active::after { content: ''; position: absolute; left: 5px; right: 5px; bottom: 0; height: 2px; border-radius: 2px; background: var(--accent); }
.workspace-header { touch-action: pan-y; }
/* xterm's vertical reader is handled in JS; horizontal drags belong to the pager. */
.term-page :deep(.xterm-screen) { touch-action: pinch-zoom; }

.fixed-brand { border: 0; padding: 0; background: transparent; color: var(--text); font: inherit; text-align: left; cursor: pointer; }
.fixed-brand:focus-visible { outline: 2px solid var(--accent); outline-offset: 4px; border-radius: 5px; }
.fixed-brand.active .m-title { color: var(--accent); }
.software-home-page { position: absolute; inset: 0; overflow-y: auto; background: var(--bg); }
.screen-trigger { width: 30px; height: 30px; padding: 6px; border: 0; border-radius: 5px; background: transparent; color: var(--text-dim); flex: none; cursor: pointer; }
.screen-trigger[aria-pressed=true] { color: var(--accent); }
.screen-trigger svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; }
.screen-trigger:focus-visible { outline: 2px solid var(--accent); }
@media (max-width: 360px) { .workspace-header:has(.screen-trigger) .m-brand { display: none; } }
</style>
