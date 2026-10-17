<script setup lang="ts">
import type { AppController, TabState } from '../state/controller'

const props = defineProps<{
  tab: TabState
  controller: AppController
}>()

</script>

<template>
  <div class="preview">
    <div class="preview-head">
      <span class="p-title">{{ tab.instanceDesktop?.name || '桌面預覽' }}</span>
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
    <div v-if="tab.desktopBusy || (tab.desktopEnabled && tab.previewVisible && !tab.shot && !tab.shotError && !tab.desktopError)" class="preview-loading" role="status" aria-live="polite"><span class="loading-spinner" aria-hidden="true" /><span>正在載入桌面，請稍候…</span></div>
    <template v-if="tab.previewVisible">
      <div v-if="tab.shot" class="shot-wrap">
        <img class="shot" :src="tab.shot.src" alt="遠端桌面截圖" />
      </div>
      <div v-else-if="!tab.desktopEnabled && !tab.desktopBusy && !tab.desktopError" class="hint small">尚無截圖</div>
      <div v-if="tab.shotError" class="error small">截圖失敗：{{ tab.shotError }}</div>
    </template>
    <div v-else-if="!tab.desktopBusy" class="hint small">已隱藏（停止截圖，遠端工作不受影響）</div>
    <div v-if="!tab.instanceDesktop && tab.desktopMsg" class="meta">{{ tab.desktopMsg }}</div>
    <div v-if="tab.desktopError" class="error small" role="alert">桌面：{{ tab.desktopError }}</div>
  </div>
</template>

<style scoped>
.preview-loading { flex: 1; display: flex; align-items: center; justify-content: center; gap: 8px; padding: 12px 4px; font-size: 11px; line-height: 1.5; color: var(--text-dim); }
.loading-spinner { flex: none; width: 16px; height: 16px; border: 2px solid var(--border); border-top-color: var(--accent); border-radius: 50%; animation: preview-spin .8s linear infinite; }
@keyframes preview-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .loading-spinner { animation: none; } }
</style>
