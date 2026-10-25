<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import type { HostDto, InstanceDesktop, Transport } from '../shared/transport/types'
import { createAppController, type AppController, type ControllerOptions, type TabState } from '../shared/state/controller'
import { TerminalFactoryKey, createDesktopXtermTerminal, type TerminalFactory } from '../shared/terminal/handle'
import { runtimeConnectedIds } from '../shared/state/runtime-entry'
import ConnectionDashboard from '../shared/components/ConnectionDashboard.vue'
import ThemeSettings from '../shared/components/ThemeSettings.vue'
import ScreenPicker from '../shared/components/ScreenPicker.vue'
import AppHomepage from '../shared/components/AppHomepage.vue'
import { selectedTheme, terminalTheme } from '../shared/theme/theme'
import '../shared/theme/themes.css'
import TabBar from '../shared/components/TabBar.vue'
import TerminalPane from '../shared/components/TerminalPane.vue'
import PreviewPanel from '../shared/components/PreviewPanel.vue'
import PreviewZoom from '../shared/components/PreviewZoom.vue'
import { useWorkspaceSize } from '../shared/components/workspace-size'

const props = defineProps<{ transport: Transport; terminalFactory?: TerminalFactory; controllerOptions?: ControllerOptions }>()
const controller: AppController = createAppController(props.transport, { ...props.controllerOptions, closeFinishedTabs: true, backgroundConnection: 'keep' })
defineExpose({ controller })
const baseFactory = props.terminalFactory ?? createDesktopXtermTerminal
provide(TerminalFactoryKey, element => {
  const terminal = baseFactory(element)
  terminal.setTheme?.(terminalTheme(selectedTheme.value))
  return terminal
})
watch(selectedTheme, theme => { for (const tab of controller.tabs) tab.term?.setTheme?.(terminalTheme(theme)) })
const activeTab = controller.activeTab
const page = ref<'workspace' | 'settings' | 'home'>('workspace')
const screenPickerOpen = ref(false), transferPicker = ref(false)
const zoomOpen = ref(false)
const settingsSection = ref<'appearance' | 'connections' | 'screens'>('appearance')
const workspace = ref<HTMLElement | null>(null)
const sizing = useWorkspaceSize(workspace)
const { sidebarWidth, previewHeight, maxWidth, maxHeight } = sizing
const documentHidden = ref(document.hidden)
const runtimeConnected = computed(() => runtimeConnectedIds(controller.tabs))
const covered = computed(() => screenPickerOpen.value)
const terminalVisible = computed(() => page.value === 'workspace' && !covered.value && !documentHidden.value)
const previewAllowed = computed(() => page.value === 'workspace' && !documentHidden.value)
const pausedPreviews = new Map<TabState, boolean>()
watch(() => [previewAllowed.value, controller.tabs.map(tab => [tab, tab.desktopEnabled, tab.previewVisible])], () => {
  for (const tab of controller.tabs) {
    if (!previewAllowed.value && tab.desktopEnabled) {
      if (!pausedPreviews.has(tab)) pausedPreviews.set(tab, tab.previewVisible)
      if (tab.previewVisible) controller.setPreviewVisible(tab, false)
    } else if (previewAllowed.value && pausedPreviews.has(tab)) {
      const restore = pausedPreviews.get(tab); pausedPreviews.delete(tab)
      if (restore && !tab.closed && tab.desktopEnabled) controller.setPreviewVisible(tab, true)
    }
  }
  for (const tab of pausedPreviews.keys()) if (tab.closed || !controller.tabs.includes(tab)) pausedPreviews.delete(tab)
}, { flush: 'post' })
function showPage(next: typeof page.value): void {
  if (transferPicker.value) return
  sizing.end()
  screenPickerOpen.value = false; page.value = next
  if (next === 'workspace') void controller.loadInstanceDesktops()
}
function showSettings(section: typeof settingsSection.value = 'appearance'): void {
  if (transferPicker.value) return
  settingsSection.value = section; showPage('settings')
}
function openTarget(host: HostDto, target: HostDto['targets'][number]): void {
  showPage('workspace'); controller.openTarget({ dto: host, expanded: true }, target, false)
  void controller.loadCatalog()
}
function settingsChanged(): void { void controller.loadCatalog(); void controller.loadInstanceDesktops() }
function openScreens(): void { screenPickerOpen.value = true; void controller.loadInstanceDesktops() }
async function selectScreen(desktop: InstanceDesktop): Promise<void> {
  const tab = activeTab.value; if (!tab) return
  pausedPreviews.delete(tab)
  screenPickerOpen.value = false
  await controller.selectInstance(tab, desktop)
}
async function clearScreen(): Promise<void> {
  const tab = activeTab.value; if (!tab) return
  pausedPreviews.delete(tab)
  screenPickerOpen.value = false
  await controller.clearInstance(tab)
}
watch(() => [activeTab.value?.key, activeTab.value?.sessionId, activeTab.value?.status], () => {
  screenPickerOpen.value = false
})
watch(terminalVisible, visible => { if (visible) void nextTick(() => activeTab.value?.term?.focus()) })
function onGlobalKey(event: KeyboardEvent): void {
  if (page.value !== 'workspace' || covered.value || documentHidden.value) return
  const target = event.target instanceof HTMLElement ? event.target : null
  if (target?.closest('input,select,[contenteditable=true]') || (target?.closest('textarea') && !target.closest('.xterm'))) return
  controller.handleKey(event)
}
function visibility(): void {
  documentHidden.value = document.hidden
  if (document.hidden) { sizing.end(); screenPickerOpen.value = false }
  void controller.setHidden(document.hidden)
}
onMounted(() => { window.addEventListener('keydown', onGlobalKey, true); document.addEventListener('visibilitychange', visibility); void controller.loadCatalog(); void controller.loadInstanceDesktops() })
onBeforeUnmount(() => { window.removeEventListener('keydown', onGlobalKey, true); document.removeEventListener('visibilitychange', visibility); controller.dispose() })
const statusText = computed(() => !activeTab.value ? '未連線' : activeTab.value.status === 'live' ? '已連線' : activeTab.value.status === 'connecting' ? '連線中…' : activeTab.value.status === 'exited' ? '已結束' : `錯誤：${activeTab.value.errorMsg}`)
</script>

<template>
  <div class="app windows-app" :data-theme="selectedTheme">
    <header class="workspace-header">
      <button class="workspace-brand" aria-label="軟體首頁" :disabled="transferPicker" @click="showPage('home')">&gt;_ <strong>art-term<span>.</span></strong></button>
      <button v-if="page !== 'workspace'" class="btn" data-testid="desktop-workspace" :disabled="transferPicker" @click="showPage('workspace')">返回工作區</button>
      <span class="spacer" />
      <button class="btn" data-testid="desktop-settings" aria-label="全域設定" :disabled="transferPicker" :aria-pressed="page === 'settings'" @click="showSettings()">⚙ 設定</button>
    </header>
    <div class="workspace-body">
      <div ref="workspace" v-show="page === 'workspace'" class="desktop-workspace" data-testid="desktop-workspace-layout">
      <aside class="desktop-sidebar" aria-label="連線與桌面" :style="{ width: `${sidebarWidth}px` }">
      <section class="desktop-main-page" aria-label="連線主畫面">
        <div class="desktop-page-heading"><div><span class="desktop-eyebrow">YOUR WORKSPACE</span><h1>連接你的工作空間</h1><p>選擇既有 Shell，或建立新的工作。</p></div></div>
        <ConnectionDashboard :transport="transport" :active="page === 'workspace' && !documentHidden" :catalog-loading="controller.catalogLoading.value" :runtime-profiles="controller.runtimeProfiles.value" :runtime-profiles-error="controller.runtimeProfilesError.value" :runtime-pending="null" :runtime-connected="runtimeConnected" :screens="controller.instanceDesktops.value" :screens-error="controller.instanceDesktopsError.value" @open="openTarget" @create="showSettings('connections')" />
      </section>
      <div class="workspace-divider horizontal" role="separator" tabindex="0" aria-label="調整桌面預覽高度" aria-orientation="horizontal" :aria-valuenow="previewHeight" :aria-valuemin="100" :aria-valuemax="maxHeight" data-testid="preview-divider" @pointerdown="sizing.start($event, 'height')" @pointermove="sizing.move" @pointerup="sizing.release" @pointercancel="sizing.release" @lostpointercapture="sizing.end" @keydown="sizing.key($event, 'height')" />
      <section class="desktop-preview" aria-label="桌面預覽" :style="{ height: `${previewHeight}px` }">
        <PreviewPanel v-if="activeTab && (activeTab.desktopEnabled || activeTab.desktopBusy || activeTab.desktopError || activeTab.shot)" :tab="activeTab" :controller="controller" @enlarge="zoomOpen = true" />
        <div v-else class="preview-empty"><strong>桌面預覽</strong><p>{{ activeTab ? '選擇 Screen 以顯示目前工作的桌面。' : '開啟連線後，可在這裡查看桌面。' }}</p><button v-if="activeTab?.status === 'live'" class="btn" @click="openScreens">選擇 Screen</button></div>
      </section>
      </aside>
      <div class="workspace-divider vertical" role="separator" tabindex="0" aria-label="調整連線側欄寬度" aria-orientation="vertical" :aria-valuenow="sidebarWidth" :aria-valuemin="160" :aria-valuemax="maxWidth" data-testid="sidebar-divider" @pointerdown="sizing.start($event, 'width')" @pointermove="sizing.move" @pointerup="sizing.release" @pointercancel="sizing.release" @lostpointercapture="sizing.end" @keydown="sizing.key($event, 'width')" />
      <section class="desktop-connect-page" aria-label="終端工作區">
        <TabBar :tabs="controller.tabs" :active-key="controller.activeKey.value" @select="tab => controller.setActive(tab.key)" @close="tab => void controller.closeTab(tab)" />
        <div v-if="activeTab" class="toolbar">
          <span class="t-name">{{ activeTab.hostLabel }} / {{ activeTab.label }}</span><span class="status" :class="activeTab.status">{{ statusText }}</span><span class="spacer" />
          <button v-if="activeTab.codingCli" class="btn" data-testid="launch-coding-cli" :disabled="activeTab.status !== 'live'" @click="controller.launchCodingCli(activeTab)">啟動 {{ activeTab.codingCli.name }}</button>
          <button v-if="activeTab.status === 'live'" class="btn screen-trigger" data-testid="desktop-toggle" aria-label="選擇 Screen" title="選擇 Screen" :aria-pressed="!!activeTab.instanceDesktop" :disabled="activeTab.desktopBusy" @click="openScreens"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="14" rx="2" /><path d="M8 21h8M12 18v3" /></svg></button>
          <button v-if="activeTab.status === 'error' || activeTab.status === 'exited'" class="btn" @click="controller.reopenTab(activeTab, false)">重新連線</button>
        </div>
        <div class="desktop-session-content">
          <div class="term-area">
            <TerminalPane v-for="tab in controller.tabs" :key="tab.key" :tab="tab" :active="terminalVisible && tab.key === controller.activeKey.value" :controller="controller" :reader-visible="terminalVisible && tab.key === controller.activeKey.value" native-scroll />
            <div v-if="!controller.tabs.length" class="empty">從左側選擇連線，開始工作。</div>
          </div>
        </div>
      </section>
      </div>
      <section v-if="page === 'settings'" class="desktop-settings-page" aria-label="全域設定"><ThemeSettings embedded desktop :initial-section="settingsSection" :transport="transport" :hosts="controller.runtimeProfiles.value" @changed="settingsChanged" @picker="transferPicker = $event" /></section>
      <section v-if="page === 'home'" class="desktop-home-page" aria-label="軟體資訊首頁"><nav class="desktop-section-nav" aria-label="軟體資訊"><span class="desktop-eyebrow">ART-TERM</span><button aria-current="page">軟體資訊與更新</button><button @click="showPage('workspace')">返回工作區</button></nav><div class="desktop-section-content"><AppHomepage platform="windows" /></div></section>
    </div>
    <footer class="statusbar"><span class="s-item" :class="activeTab?.status">{{ statusText }}</span><span v-if="activeTab?.remoteSession" class="s-item">{{ activeTab.remoteSession }}</span><span v-if="activeTab?.notice" class="s-item warn">{{ activeTab.notice }}</span><span v-if="controller.uiNotice.value" class="s-item warn">{{ controller.uiNotice.value }}</span><span class="spacer" /><span class="s-item muted">{{ controller.tabs.length }} / 32 分頁</span></footer>
    <PreviewZoom v-if="zoomOpen && page === 'workspace' && activeTab && activeTab.previewVisible && (activeTab.desktopEnabled || activeTab.shot)" :tab="activeTab" :screen-key="`${activeTab.key}:${activeTab.instanceDesktop?.id || 'legacy'}`" mode="windows" @close="zoomOpen = false" />
    <ScreenPicker v-if="screenPickerOpen && activeTab" :items="controller.instanceDesktops.value" :selected-id="activeTab.instanceDesktop?.id" :busy="activeTab.desktopBusy" :error="activeTab.desktopError || controller.instanceDesktopsError.value" @select="selectScreen" @clear="clearScreen" @close="screenPickerOpen = false" @settings="showSettings('screens')" />
  </div>
</template>

<style scoped>
.windows-app { position: relative; overflow: hidden; }
.desktop-workspace { display: flex; height: 100%; min-height: 0; }
.desktop-sidebar { flex: none; min-width: 0; display: flex; flex-direction: column; min-height: 0; overflow: hidden; background: var(--bg-panel); }
.desktop-sidebar .desktop-main-page { flex: 1; min-height: 0; height: auto; padding: 12px 8px; margin: 0; overflow-x: hidden; overflow-y: auto; }
.desktop-sidebar .desktop-page-heading { margin-bottom: 16px; }
.desktop-sidebar h1 { font-size: 17px; }
.desktop-sidebar .desktop-page-heading p { font-size: 11px; line-height: 1.5; }
.desktop-sidebar :deep(.dashboard) { display: flex; flex-direction: column; gap: 12px; align-items: stretch; }
.desktop-sidebar .desktop-preview { flex: none; width: auto; min-width: 0; min-height: 0; max-height: none; padding: 8px; border: 0; overflow: hidden; }
.desktop-sidebar :deep(.preview) { display: flex; flex-direction: column; height: 100%; max-height: none; min-height: 0; overflow: hidden; border-top: 0; padding: 0; }
.desktop-sidebar :deep(.preview-head) { flex: none; flex-wrap: wrap; gap: 4px; padding: 0 0 8px; }
.desktop-sidebar :deep(.preview-head .p-title) { flex: 1 0 100%; overflow-wrap: anywhere; }
.desktop-sidebar :deep(.shot-wrap) { display: flex; flex: 1; min-height: 0; padding: 0; background: #000; align-items: center; justify-content: center; }
.desktop-sidebar :deep(.shot) { width: 100%; height: 100%; object-fit: contain; object-position: center; border: 0; border-radius: 0; }
.desktop-connect-page { flex: 1; min-width: 0; }
.preview-empty p { color: var(--text-dim); line-height: 1.6; }
.workspace-divider { flex: none; background: var(--border); touch-action: none; user-select: none; position: relative; z-index: 2; }
.workspace-divider:hover,.workspace-divider:focus-visible { background: var(--accent); outline: none; }
.workspace-divider.vertical { width: 5px; cursor: col-resize; }
.workspace-divider.horizontal { height: 5px; cursor: row-resize; }
.screen-trigger svg { display: block; width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; }
.desktop-sidebar :deep(.section-heading) { padding-inline: 8px; }
.desktop-sidebar :deep(.section-content) { margin-inline: 6px; }
.desktop-sidebar :deep(.category-toggle) { gap: 5px; }
.desktop-sidebar :deep(.section-icon) { width: 24px; height: 24px; }
.desktop-sidebar :deep(.host-connection-grid) { grid-template-columns: minmax(0, 1fr) 32px; gap: 4px; }
.desktop-sidebar :deep(.host-identity) { grid-column: 1; }
.desktop-sidebar :deep(.new-connection) { grid-column: 2; grid-row: 1; }
.desktop-sidebar :deep(.existing-connections) { grid-column: 1 / -1; grid-row: 2; }
.desktop-sidebar :deep(.host-copy small) { overflow-wrap: anywhere; }
</style>
