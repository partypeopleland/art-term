<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import type { HostDto, InstanceDesktop, Transport } from './transport/types'
import { createAppController, type AppController, type ControllerOptions, type TabState } from './state/controller'
import { TerminalFactoryKey, createDesktopXtermTerminal, type TerminalFactory } from './terminal/handle'
import { runtimeConnectedIds } from './state/runtime-entry'
import ConnectionDashboard from './mobile/ConnectionDashboard.vue'
import ThemeSettings from './mobile/ThemeSettings.vue'
import ScreenPicker from './mobile/ScreenPicker.vue'
import AppHomepage from './mobile/AppHomepage.vue'
import { selectedTheme, terminalTheme } from './mobile/theme'
import './mobile/themes.css'
import TabBar from './components/TabBar.vue'
import TerminalPane from './components/TerminalPane.vue'
import PreviewPanel from './components/PreviewPanel.vue'
import RuntimeHosts from './components/RuntimeHosts.vue'

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
const hostsOpen = ref(false), screenPickerOpen = ref(false), transferPicker = ref(false)
const hostsView = ref<'list' | 'create'>('list')
const hostsCategory = ref<'host' | 'agent' | undefined>()
const documentHidden = ref(document.hidden)
const runtimeConnected = computed(() => runtimeConnectedIds(controller.tabs))
const covered = computed(() => hostsOpen.value || screenPickerOpen.value)
const terminalVisible = computed(() => page.value === 'workspace' && !covered.value && !documentHidden.value)
const previewAllowed = computed(() => page.value === 'workspace' && !hostsOpen.value && !documentHidden.value)
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
  screenPickerOpen.value = false; hostsOpen.value = false; page.value = next
  if (next === 'workspace') void controller.loadInstanceDesktops()
}
function manageHosts(view: 'list' | 'create', category?: 'host' | 'agent'): void {
  hostsView.value = view; hostsCategory.value = category; hostsOpen.value = true
}
function openTarget(host: HostDto, target: HostDto['targets'][number]): void {
  hostsOpen.value = false; showPage('workspace'); controller.openTarget({ dto: host, expanded: true }, target, false)
  void controller.loadCatalog()
}
function settingsChanged(): void { void controller.loadCatalog(); void controller.loadInstanceDesktops() }
function openScreens(): void { screenPickerOpen.value = true; void controller.loadInstanceDesktops() }
async function selectScreen(desktop: InstanceDesktop): Promise<void> {
  const tab = activeTab.value; if (!tab) return
  pausedPreviews.delete(tab)
  await controller.selectInstance(tab, desktop)
  if (activeTab.value === tab && tab.instanceDesktop?.id === desktop.id) screenPickerOpen.value = false
}
async function clearScreen(): Promise<void> {
  const tab = activeTab.value; if (!tab) return
  pausedPreviews.delete(tab)
  await controller.clearInstance(tab); if (activeTab.value === tab) screenPickerOpen.value = false
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
  if (document.hidden) { if (!transferPicker.value) hostsOpen.value = false; screenPickerOpen.value = false }
  void controller.setHidden(document.hidden)
}
watch(hostsOpen, open => { if (!open) transferPicker.value = false })
onMounted(() => { window.addEventListener('keydown', onGlobalKey, true); document.addEventListener('visibilitychange', visibility); void controller.loadCatalog(); void controller.loadInstanceDesktops() })
onBeforeUnmount(() => { window.removeEventListener('keydown', onGlobalKey, true); document.removeEventListener('visibilitychange', visibility); controller.dispose() })
const statusText = computed(() => !activeTab.value ? '未連線' : activeTab.value.status === 'live' ? '已連線' : activeTab.value.status === 'connecting' ? '連線中…' : activeTab.value.status === 'exited' ? '已結束' : `錯誤：${activeTab.value.errorMsg}`)
</script>

<template>
  <div class="app windows-app" :data-theme="selectedTheme">
    <header class="workspace-header">
      <button class="workspace-brand" aria-label="軟體首頁" @click="showPage('home')">&gt;_ <strong>art-term<span>.</span></strong></button>
      <button v-if="page !== 'workspace'" class="btn" data-testid="desktop-workspace" @click="showPage('workspace')">返回工作區</button>
      <span class="spacer" />
      <button class="btn" data-testid="desktop-settings" aria-label="全域設定" :aria-pressed="page === 'settings'" @click="showPage('settings')">⚙ 設定</button>
    </header>
    <div class="workspace-body">
      <div v-show="page === 'workspace'" class="desktop-workspace" data-testid="desktop-workspace-layout">
      <aside class="desktop-sidebar" aria-label="連線與桌面">
      <section class="desktop-main-page" aria-label="連線主畫面">
        <div class="desktop-page-heading"><div><span class="desktop-eyebrow">YOUR WORKSPACE</span><h1>連接你的工作空間</h1><p>選擇既有 Shell，或建立新的工作。</p></div></div>
        <ConnectionDashboard :transport="transport" :active="page === 'workspace' && !hostsOpen && !documentHidden" :catalog-loading="controller.catalogLoading.value" :runtime-profiles="controller.runtimeProfiles.value" :runtime-profiles-error="controller.runtimeProfilesError.value" :runtime-pending="null" :runtime-connected="runtimeConnected" :screens="controller.instanceDesktops.value" :screens-error="controller.instanceDesktopsError.value" @open="openTarget" @create="category => manageHosts('create', category)" />
      </section>
      <section class="desktop-preview" aria-label="桌面預覽">
        <PreviewPanel v-if="activeTab && (activeTab.desktopEnabled || activeTab.desktopBusy || activeTab.desktopError)" :tab="activeTab" :controller="controller" />
        <div v-else class="preview-empty"><strong>桌面預覽</strong><p>{{ activeTab ? '選擇 Screen 以顯示目前工作的桌面。' : '開啟連線後，可在這裡查看桌面。' }}</p><button v-if="activeTab?.status === 'live'" class="btn" @click="openScreens">選擇 Screen</button></div>
      </section>
      </aside>
      <section class="desktop-connect-page" aria-label="終端工作區">
        <TabBar :tabs="controller.tabs" :active-key="controller.activeKey.value" @select="tab => controller.setActive(tab.key)" @close="tab => void controller.closeTab(tab)" />
        <div v-if="activeTab" class="toolbar">
          <span class="t-name">{{ activeTab.hostLabel }} / {{ activeTab.label }}</span><span class="status" :class="activeTab.status">{{ statusText }}</span><span class="spacer" />
          <button v-if="activeTab.codingCli" class="btn" data-testid="launch-coding-cli" :disabled="activeTab.status !== 'live'" @click="controller.launchCodingCli(activeTab)">啟動 {{ activeTab.codingCli.name }}</button>
          <button v-if="activeTab.status === 'live'" class="btn" data-testid="desktop-toggle" :aria-pressed="!!activeTab.instanceDesktop" :disabled="activeTab.desktopBusy" @click="openScreens">{{ activeTab.instanceDesktop?.name || '選擇 Screen' }}</button>
          <button v-if="activeTab.status === 'error' || activeTab.status === 'exited'" class="btn" @click="controller.reopenTab(activeTab, false)">重新連線</button>
        </div>
        <div class="desktop-session-content">
          <div class="term-area">
            <TerminalPane v-for="tab in controller.tabs" :key="tab.key" :tab="tab" :active="terminalVisible && tab.key === controller.activeKey.value" :controller="controller" inline-history />
            <div v-if="!controller.tabs.length" class="empty">從左側選擇連線，開始工作。</div>
          </div>
        </div>
      </section>
      </div>
      <section v-if="page === 'settings'" class="desktop-settings-page" aria-label="全域設定"><div class="desktop-settings-actions"><button class="btn" data-testid="desktop-manage-hosts" @click="manageHosts('list')">管理既有連線（編輯／測試／刪除）</button></div><ThemeSettings v-if="!hostsOpen" embedded :transport="transport" :hosts="controller.runtimeProfiles.value" @changed="settingsChanged" @picker="transferPicker = $event" /></section>
      <section v-if="page === 'home'" class="desktop-home-page" aria-label="軟體資訊首頁"><AppHomepage platform="windows" /></section>
    </div>
    <footer class="statusbar"><span class="s-item" :class="activeTab?.status">{{ statusText }}</span><span v-if="activeTab?.remoteSession" class="s-item">{{ activeTab.remoteSession }}</span><span v-if="activeTab?.notice" class="s-item warn">{{ activeTab.notice }}</span><span v-if="controller.uiNotice.value" class="s-item warn">{{ controller.uiNotice.value }}</span><span class="spacer" /><span class="s-item muted">{{ controller.tabs.length }} / 32 分頁</span></footer>
    <ScreenPicker v-if="screenPickerOpen && activeTab" :items="controller.instanceDesktops.value" :selected-id="activeTab.instanceDesktop?.id" :busy="activeTab.desktopBusy" :error="activeTab.desktopError || controller.instanceDesktopsError.value" @select="selectScreen" @clear="clearScreen" @close="screenPickerOpen = false" @settings="showPage('settings')" />
    <RuntimeHosts v-if="hostsOpen" :transport="transport" :initial-view="hostsView" :initial-category="hostsCategory" @close="hostsOpen = false" @changed="settingsChanged" @open="openTarget" @picker="transferPicker = $event" />
  </div>
</template>

<style scoped>
.desktop-workspace { display: flex; height: 100%; min-height: 0; }
.desktop-sidebar { flex: 0 0 33.333%; min-width: 260px; display: flex; flex-direction: column; border-right: 1px solid var(--border); min-height: 0; }
.desktop-sidebar .desktop-main-page { flex: 1; min-height: 0; height: auto; padding: 16px; margin: 0; overflow: auto; }
.desktop-sidebar .desktop-page-heading { margin-bottom: 16px; }
.desktop-sidebar h1 { font-size: 20px; }
.desktop-sidebar :deep(.dashboard) { display: flex; flex-direction: column; gap: 12px; align-items: stretch; }
.desktop-sidebar .desktop-preview { flex: 0 0 30%; width: auto; min-width: 0; min-height: 150px; max-height: 45%; padding: 12px; border-left: 0; border-top: 1px solid var(--border); overflow: auto; }
.desktop-sidebar :deep(.preview) { display: flex; flex-direction: column; height: 100%; max-height: none; min-height: 0; overflow: hidden; border-top: 0; padding: 0; }
.desktop-sidebar :deep(.preview-head) { flex: none; padding: 0 0 6px; }
.desktop-sidebar :deep(.shot-wrap) { flex: 1; min-height: 0; padding: 0; }
.desktop-sidebar :deep(.shot) { height: 100%; object-fit: contain; }
.desktop-connect-page { flex: 1; min-width: 0; }
.preview-empty p { color: var(--text-dim); line-height: 1.6; }
</style>
