<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, ref } from 'vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from './transport/types'
import {
  createAppController,
  type AppController,
  type ControllerOptions,
  type TabState
} from './state/controller'
import {
  TerminalFactoryKey,
  createDesktopXtermTerminal,
  type TerminalFactory
} from './terminal/handle'
import { createRuntimeEntry, runtimeConnectedIds } from './state/runtime-entry'
import CategoryList from './components/CategoryList.vue'
import TabBar from './components/TabBar.vue'
import TerminalPane from './components/TerminalPane.vue'
import PreviewPanel from './components/PreviewPanel.vue'
import RuntimeHosts from './components/RuntimeHosts.vue'
import RuntimeConnectionDialog from './components/RuntimeConnectionDialog.vue'

const props = defineProps<{
  transport: Transport
  terminalFactory?: TerminalFactory
  controllerOptions?: ControllerOptions
}>()

// Composition happens here, but Tauri itself is only imported by main.ts —
// tests mount this same App with a fake transport and fake terminal factory.
provide(TerminalFactoryKey, props.terminalFactory ?? createDesktopXtermTerminal)

const controller: AppController = createAppController(props.transport, props.controllerOptions ?? {})
defineExpose({ controller })

const activeTab = controller.activeTab
const hostsOpen = ref(false)
const hostsView = ref<'list' | 'create'>('list')
function manageHosts(view: 'list' | 'create'): void { runtimeEntry.cancel(); hostsView.value = view; hostsOpen.value = true }
const runtimeDialogProfile = ref<RuntimeHostDto | null>(null)
const runtimeDialogSeed = ref<HostDto | null>(null)

function openRuntimeEntry(profile: RuntimeHostDto, seed: HostDto | null = null): void {
  // Frozen snapshot: a later catalog reload cannot swap the id/revision the
  // dialog lists or creates sessions against.
  runtimeDialogSeed.value = seed
  runtimeDialogProfile.value = { ...profile }
}

function openRuntimeTarget(host: HostDto, target: TargetDto): void {
  runtimeDialogProfile.value = null
  runtimeDialogSeed.value = null
  controller.openTarget({ dto: host, expanded: true }, target, false)
}

// Green dot input: local live runtime tabs only — never a remote probe.
const runtimeConnected = computed(() => runtimeConnectedIds(controller.tabs))

// Shared entry behaviour: primary click reuses a live same-host tab, else
// one bounded list with a single-running-ready-Shell fast path; the right
// Shell button always opens the chooser.
const runtimeEntry = createRuntimeEntry(props.transport, controller, {
  activateTab: (tab) => controller.setActive(tab.key),
  openChooser: (profile, seed) => openRuntimeEntry(profile, seed),
  openTarget: openRuntimeTarget,
  showError: (message) => {
    controller.uiNotice.value = message
  }
})

function onGlobalKey(e: KeyboardEvent): void {
  // Capture phase: runs before xterm's own keydown handler, so Alt+[/Alt+]
  // and Ctrl+PgUp/PgDn never reach the remote as escape bytes. Everything
  // else falls through to the focused terminal unchanged.
  controller.handleKey(e)
}

onMounted(() => {
  window.addEventListener('keydown', onGlobalKey, true)
  void controller.loadCatalog()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onGlobalKey, true)
  runtimeEntry.dispose()
  controller.dispose()
})

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
      return `錯誤：${t.errorMsg}`
  }
})

function desktopToggleLabel(tab: TabState): string {
  if (tab.desktopBusy) return '桌面切換中…'
  return tab.desktopEnabled ? '停用桌面' : '啟用桌面'
}
</script>

<template>
  <div class="app">
    <aside class="side">
      <CategoryList
        :catalog-loading="controller.catalogLoading.value"
        :runtime-profiles="controller.runtimeProfiles.value"
        :runtime-profiles-error="controller.runtimeProfilesError.value"
        :runtime-pending="runtimeEntry.pendingId.value"
        :runtime-connected="runtimeConnected"
        @runtime-open="runtimeEntry.primary"
        @runtime-shell="runtimeEntry.shell"
        @create="manageHosts('create')"
        @manage="manageHosts('list')"
        @refresh="controller.loadCatalog()"
      />
      <PreviewPanel
        v-if="activeTab && (activeTab.desktopEnabled || activeTab.desktopBusy || activeTab.desktopError)"
        :tab="activeTab"
        :controller="controller"
      />
    </aside>

    <main class="main">
      <TabBar
        :tabs="controller.tabs"
        :active-key="controller.activeKey.value"
        @select="(t) => { runtimeEntry.cancel(); controller.setActive(t.key) }"
        @close="(t) => void controller.closeTab(t)"
      />

      <div v-if="activeTab" class="toolbar">
        <span class="t-name">{{ activeTab.hostLabel }} / {{ activeTab.label }}</span>
        <span class="status" :class="activeTab.status">{{ statusText }}</span>
        <span class="spacer" />
        <button v-if="activeTab.codingCli" class="btn" data-testid="launch-coding-cli"
          :disabled="activeTab.status !== 'live'" @click="controller.launchCodingCli(activeTab)">啟動 {{ activeTab.codingCli.name }}</button>
        <button
          v-if="activeTab.status === 'live' && activeTab.desktopAvailable"
          class="btn"
          :disabled="activeTab.desktopBusy"
          @click="controller.setDesktop(activeTab, !activeTab.desktopEnabled)"
        >{{ desktopToggleLabel(activeTab) }}</button>
        <button
          v-if="activeTab.status === 'exited' || activeTab.status === 'error'"
          class="btn"
          @click="controller.reopenTab(activeTab, false)"
        >重新連線</button>
      </div>

      <div class="term-area">
        <TerminalPane
          v-for="tab in controller.tabs"
          :key="tab.key"
          :tab="tab"
          :active="tab.key === controller.activeKey.value"
          :controller="controller"
        />
        <div v-if="!controller.tabs.length" class="empty">
          <p>尚未開啟連線。</p>
          <p class="muted">在左側選擇連線名稱，或按「Shell」選擇及建立 Shell。</p>
        </div>
      </div>

      <footer class="statusbar">
        <span v-if="activeTab?.remoteSession" class="s-item">{{ activeTab.remoteSession }}</span>
        <span v-if="activeTab" class="s-item">{{ activeTab.hostLabel }} / {{ activeTab.label }}</span>
        <span v-if="activeTab" class="s-item" :class="activeTab.status">{{ statusText }}</span>
        <span v-if="activeTab?.desktopEnabled" class="s-item">桌面已啟用</span>
        <span v-if="activeTab?.notice" class="s-item warn">{{ activeTab.notice }}</span>
        <span v-if="controller.uiNotice.value" class="s-item warn">{{ controller.uiNotice.value }}</span>
        <span class="spacer" />
        <span class="s-item muted">{{ controller.tabs.length }} / 32 分頁</span>
      </footer>
    </main>
    <RuntimeHosts v-if="hostsOpen" :transport="transport" :initial-view="hostsView" @close="hostsOpen = false" @changed="controller.loadCatalog()" @open="(host, target) => { hostsOpen = false; controller.openTarget({ dto: host, expanded: true }, target, false) }" />
    <RuntimeConnectionDialog v-if="runtimeDialogProfile" :profile="runtimeDialogProfile" :transport="transport" :initial-group="runtimeDialogSeed" @open="openRuntimeTarget" @close="runtimeDialogProfile = null; runtimeDialogSeed = null" />
  </div>
</template>
