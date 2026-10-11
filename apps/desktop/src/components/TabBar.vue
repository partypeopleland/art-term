<script setup lang="ts">
import type { TabState } from '../state/controller'

defineProps<{
  tabs: TabState[]
  activeKey: string | null
}>()

const emit = defineEmits<{
  select: [tab: TabState]
  close: [tab: TabState]
}>()

function statusGlyph(tab: TabState): string {
  if (tab.status === 'live') return '●'
  if (tab.status === 'connecting') return '…'
  return '×'
}
</script>

<template>
  <div class="tabbar" role="tablist">
    <div
      v-for="tab in tabs"
      :key="tab.key"
      class="tab"
      :class="{ active: tab.key === activeKey }"
      role="tab"
      :title="tab.remoteSession ? `${tab.hostLabel} / ${tab.label}（${tab.remoteSession}）` : `${tab.hostLabel} / ${tab.label}`"
      @click="emit('select', tab)"
    >
      <span class="glyph" :class="tab.status">{{ statusGlyph(tab) }}</span>
      <span class="name">{{ tab.label }}</span>
      <span v-if="tab.desktopEnabled" class="desk" title="虛擬桌面已啟用">◈</span>
      <button class="close" title="關閉分頁（僅中斷本機連線）" @click.stop="emit('close', tab)">×</button>
    </div>
    <div v-if="!tabs.length" class="hint">從左側選擇目標建立連線（Alt+[ / Alt+] 或 Ctrl+PgUp/PgDn 切換分頁）</div>
  </div>
</template>
