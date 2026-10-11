<script setup lang="ts">
import type { AppController, TabState } from '../state/controller'

const props = defineProps<{
  tab: TabState
  controller: AppController
}>()

function fmtTime(ms: number): string {
  try {
    return new Date(ms).toLocaleTimeString('zh-TW', { hour12: false })
  } catch {
    return String(ms)
  }
}
</script>

<template>
  <div class="preview">
    <div class="preview-head">
      <span class="p-title">桌面預覽</span>
      <button
        class="mini"
        :disabled="!tab.previewVisible || tab.desktopBusy"
        title="立即重新截圖"
        @click="props.controller.manualCapture()"
      >⟳ 更新</button>
      <button
        class="mini"
        data-testid="preview-visibility"
        @click="props.controller.setPreviewVisible(tab, !tab.previewVisible)"
      >{{ tab.previewVisible ? '隱藏' : '顯示' }}</button>
    </div>
    <div v-if="tab.desktopBusy" class="hint small" role="status">正在更新桌面預覽設定…</div>
    <template v-if="tab.previewVisible">
      <div v-if="tab.shot" class="shot-wrap">
        <img class="shot" :src="tab.shot.src" alt="遠端桌面截圖" />
      </div>
      <div v-else-if="!tab.desktopBusy && !tab.desktopError" class="hint small">尚無截圖</div>
      <div v-if="tab.shot" class="meta">{{ tab.shot.source }} · {{ fmtTime(tab.shot.capturedAt) }}</div>
      <div v-if="tab.shotError" class="error small">截圖失敗：{{ tab.shotError }}</div>
    </template>
    <div v-else class="hint small">已隱藏（停止截圖，遠端工作不受影響）</div>
    <div v-if="tab.desktopMsg" class="meta">{{ tab.desktopMsg }}</div>
    <div v-if="tab.desktopError" class="error small" role="alert">桌面：{{ tab.desktopError }}</div>
  </div>
</template>
