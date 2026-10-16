<script setup lang="ts">
import { ref } from 'vue'
import { useModalFocus } from '../components/modal-focus'
import { themes, selectedTheme, selectTheme } from './theme'
import RuntimeTransfer from '../components/RuntimeTransfer.vue'
import RuntimeHosts from '../components/RuntimeHosts.vue'
import type { RuntimeHostDto, Transport } from '../transport/types'
const props = withDefaults(defineProps<{ embedded?: boolean; transport?: Transport; hosts?: RuntimeHostDto[] }>(), { hosts: () => [] })
const emit = defineEmits<{ close: []; changed: []; picker: [active: boolean] }>()
const transferMode = ref<'import' | 'export' | null>(null)
const dialog = ref<HTMLElement | null>(null)
if (!props.embedded) useModalFocus(dialog, () => emit('close'))
</script>

<template>
  <div class="settings-backdrop" :class="{ 'embedded-settings-page': embedded }" @click.self="emit('close')">
    <section ref="dialog" class="settings-panel" :role="embedded ? 'region' : 'dialog'" :aria-modal="embedded ? undefined : true" aria-labelledby="settings-title" tabindex="-1">
      <header><div><span class="settings-eyebrow">GLOBAL SETTINGS</span><h2 id="settings-title">外觀設定</h2></div><button v-if="!embedded" class="settings-close" aria-label="關閉設定" @click="emit('close')">×</button></header>
      <div class="settings-label">THEME <span>套用至所有頁面</span></div>
      <div class="theme-options" role="group" aria-label="Theme">
        <button v-for="theme in themes" :key="theme.id" class="theme-option" :class="{ selected: selectedTheme === theme.id }" :aria-pressed="selectedTheme === theme.id" @click="selectTheme(theme.id)">
          <span class="theme-swatches" aria-hidden="true"><i v-for="color in theme.colors" :key="color" :style="{ background: color }" /></span>
          <span class="theme-copy"><strong>{{ theme.name }}</strong><small>{{ theme.description }}</small></span><span class="theme-check" aria-hidden="true">{{ selectedTheme === theme.id ? '✓' : '' }}</span>
        </button>
      </div>
      <p class="settings-note">立即生效，自動記住此裝置的選擇。</p>
      <section v-if="transport" class="connection-transfer-settings" aria-labelledby="transfer-settings-title">
        <div class="settings-eyebrow">CONNECTIONS</div>
        <h2 id="transfer-settings-title">連線設定</h2>
        <div class="transfer-setting-actions">
          <button :aria-pressed="transferMode === 'import'" @click="transferMode = 'import'"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m-4-4 4 4 4-4M4 16v4h16v-4" /></svg><span>匯入連線</span></button>
          <button :disabled="!hosts.length" :aria-pressed="transferMode === 'export'" @click="transferMode = 'export'"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 15V3m-4 4 4-4 4 4M4 16v4h16v-4" /></svg><span>匯出連線</span></button>
        </div>
        <RuntimeTransfer v-if="transferMode" :key="transferMode" :mode="transferMode" :transport="transport" :hosts="hosts" @close="transferMode = null" @changed="emit('changed')" @picker="emit('picker', $event)" />
        <div class="new-connection-settings">
          <RuntimeHosts embedded :transport="transport" initial-view="create" @changed="emit('changed')" />
        </div>
      </section>
    </section>
  </div>
</template>

<style scoped>
.settings-backdrop { position: absolute; inset: 0; z-index: 150; display: flex; align-items: flex-start; justify-content: center; padding: 72px 16px 20px; background: #0007; overflow: auto; }
.settings-panel { width: 100%; max-width: 420px; padding: 18px; background: var(--bg-panel); color: var(--text); border: 1px solid var(--border); border-radius: 15px; box-shadow: 0 20px 70px #0005; }
header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 22px; }
.settings-eyebrow { font-size: 9px; letter-spacing: .14em; color: var(--text-dim); }
h2 { margin: 5px 0 0; font-size: 18px; font-weight: 600; }
button { font: inherit; cursor: pointer; }
button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.settings-close { width: 44px; height: 44px; border: 0; border-radius: 8px; background: var(--bg-raised); color: var(--text-dim); font-size: 24px; }
.settings-label { display: flex; justify-content: space-between; margin-bottom: 10px; font-size: 10px; letter-spacing: .08em; color: var(--text-dim); }
.settings-label span { letter-spacing: 0; }
.theme-options { display: grid; gap: 8px; }
.theme-option { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 66px; padding: 11px; background: var(--bg); border: 1px solid var(--border); border-radius: 10px; color: var(--text); text-align: left; }
.theme-option.selected { border-color: var(--accent); background: var(--bg-raised); }
.theme-swatches { display: grid; grid-template-columns: repeat(2, 14px); gap: 3px; flex: none; }
.theme-swatches i { width: 14px; height: 14px; border-radius: 4px; box-shadow: inset 0 0 0 1px #8883; }
.theme-copy { display: flex; flex-direction: column; gap: 4px; flex: 1; }
.theme-copy strong { font-size: 12px; font-weight: 600; }
.theme-copy small { font-size: 10px; color: var(--text-dim); }
.theme-check { width: 16px; color: var(--accent); }
.settings-note { margin: 15px 0 0; font-size: 10px; color: var(--text-dim); text-align: center; }

.connection-transfer-settings { margin-top: 28px; padding-top: 22px; border-top: 1px solid var(--border); }
.transfer-setting-actions { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-top: 14px; }
.transfer-setting-actions button { display: flex; align-items: center; justify-content: center; gap: 8px; min-height: 48px; padding: 10px; border: 1px solid var(--border); border-radius: 9px; color: var(--text); background: var(--bg-panel); font-size: 12px; }
.transfer-setting-actions button[aria-pressed="true"] { color: var(--accent); border-color: var(--accent); }
.transfer-setting-actions button:disabled { opacity: .4; cursor: default; }
.transfer-setting-actions svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.connection-transfer-settings :deep(.runtime-transfer) { margin-top: 16px; }

.settings-backdrop.embedded-settings-page { position: static; display: block; padding: 14px; background: transparent; overflow: visible; }
.embedded-settings-page .settings-panel { margin: 0 auto; max-width: 600px; box-shadow: none; border-radius: 12px; }

.settings-backdrop:not(.embedded-settings-page) { position: absolute; inset: 0; z-index: 150; display: block; padding: 0; background: var(--bg); overflow-y: auto; }
.settings-backdrop:not(.embedded-settings-page) .settings-panel { width: 100%; max-width: none; min-height: 100%; padding: 20px 16px; border: 0; border-radius: 0; box-shadow: none; background: var(--bg); }
.settings-backdrop:not(.embedded-settings-page) header { margin-bottom: 24px; }
.settings-backdrop:not(.embedded-settings-page) .settings-close { background: transparent; }

.embedded-settings-page .settings-panel { border: 0; border-radius: 0; padding: 6px 0 24px; background: transparent; box-shadow: none; }
.embedded-settings-page { min-height: 100%; }

.new-connection-settings { margin-top: 22px; padding-top: 20px; border-top: 1px solid var(--border); }
.new-connection-settings :deep(.runtime-host-backdrop.embedded-host-page.embedded-host-page) { padding: 0; }
.new-connection-settings :deep(.embedded-host-page.embedded-host-page .runtime-hosts) { padding: 0; border: 0; border-radius: 0; background: transparent; }
.new-connection-settings :deep(.runtime-hosts h2) { font-size: 14px; }
</style>
