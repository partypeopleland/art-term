<script setup lang="ts">
import { ref } from 'vue'
import type { Transport, UpdateCheckDto } from '../transport/types'

const props = defineProps<{ transport: Transport }>()
const checking = ref(false)
const installing = ref(false)
const result = ref<UpdateCheckDto | null>(null)
const error = ref('')
const notice = ref('')

function messageOf(errorValue: unknown): string {
  const message = errorValue instanceof Error ? errorValue.message : String(errorValue)
  return message.replace(/\s+/g, ' ').trim().slice(0, 300)
}

async function check(): Promise<void> {
  if (!props.transport.updateCheck || checking.value || installing.value) return
  checking.value = true
  error.value = ''
  notice.value = ''
  try {
    result.value = await props.transport.updateCheck()
  } catch (value) {
    error.value = messageOf(value)
  } finally {
    checking.value = false
  }
}

async function install(): Promise<void> {
  if (!props.transport.updateInstall || !result.value?.available || installing.value) return
  installing.value = true
  error.value = ''
  notice.value = ''
  try {
    const outcome = await props.transport.updateInstall(result.value.latest)
    notice.value = outcome.status === 'installed'
      ? `已安裝 ${outcome.version ?? result.value.latest}`
      : '已驗證下載內容，正在啟動系統安裝程式；請在系統提示完成安裝。'
  } catch (value) {
    error.value = messageOf(value)
  } finally {
    installing.value = false
  }
}
</script>

<template>
  <section class="update-settings" aria-labelledby="update-settings-title">
    <div class="settings-eyebrow">SOFTWARE UPDATE</div>
    <h2 id="update-settings-title">軟體更新</h2>
    <p class="update-note">只在按下檢查時連線 GitHub Release；下載後先驗證完整性，再交給系統安裝。</p>
    <div class="update-actions">
      <button type="button" data-testid="update-check" :disabled="checking || installing" @click="check">
        {{ checking ? '檢查中…' : '檢查 GitHub 更新' }}
      </button>
      <button v-if="result?.available" type="button" data-testid="update-install" :disabled="installing" @click="install">
        {{ installing ? '下載安裝中…' : `下載並安裝 ${result.latest}` }}
      </button>
    </div>
    <p v-if="result" class="update-result" role="status">目前版本 {{ result.current }}；最新版本 {{ result.latest }}。{{ result.message }}</p>
    <p v-if="notice" class="update-result" role="status">{{ notice }}</p>
    <p v-if="error" class="update-error" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.update-settings { margin-top: 22px; padding-top: 20px; border-top: 1px solid var(--border); }
.update-note { margin: 10px 0 0; color: var(--text-dim); font-size: 11px; line-height: 1.6; }
.update-actions { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 14px; }
.update-actions button { min-height: 44px; padding: 10px 14px; border: 1px solid var(--border); border-radius: 9px; color: var(--text); background: var(--bg-panel); font-size: 12px; cursor: pointer; }
.update-actions button:disabled { opacity: .45; cursor: default; }
.update-result { margin: 12px 0 0; color: var(--text-dim); font-size: 11px; line-height: 1.6; }
.update-error { margin: 12px 0 0; color: var(--danger, #d66); font-size: 11px; line-height: 1.6; }
</style>
