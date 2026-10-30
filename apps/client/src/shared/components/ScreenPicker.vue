<script setup lang="ts">
import { ref } from 'vue'
import { useModalFocus } from './modal-focus'
import type { InstanceDesktop } from '../transport/types'
defineProps<{ items: InstanceDesktop[]; selectedId?: string; busy: boolean; error?: string }>()
const emit = defineEmits<{ select: [desktop: InstanceDesktop]; clear: []; close: []; settings: [] }>()
const dialog = ref<HTMLElement | null>(null)
useModalFocus(dialog, () => emit('close'))
</script>
<template>
  <div class="screen-picker-backdrop"><section ref="dialog" class="screen-picker" role="dialog" aria-modal="true" aria-label="選擇 Screen" tabindex="-1" data-testid="screen-picker">
    <header><h2>選擇 Screen</h2><button aria-label="關閉 Screen 選單" @click="emit('close')">×</button></header>
    <p v-if="!items.length">尚未設定 Screen，請到齒輪的連線設定新增。</p>
    <button v-for="item in items" :key="item.id" class="screen-choice" :data-testid="`choose-screen-${item.id}`" :disabled="busy" :aria-pressed="selectedId === item.id" @click="emit('select', item)"><strong>{{ item.name }}</strong><span>{{ item.mcpUrl }}</span></button>
    <p v-if="error" role="alert">{{ error }}</p>
    <footer><button v-if="selectedId" :disabled="busy" data-testid="clear-screen" @click="emit('clear')">取消畫面與工具</button><button @click="emit('settings')">連線設定</button></footer>
  </section></div>
</template>
<style scoped>
.screen-picker-backdrop { position: absolute; inset: 0; z-index: 120; display: flex; align-items: center; justify-content: center; padding: 18px; background: #0007; }
.screen-picker { width: 100%; max-width: 440px; max-height: 75%; overflow-y: auto; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-panel); color: var(--text); padding: 16px; }
header { display: flex; align-items: center; justify-content: space-between; } h2 { margin: 0; font-size: 16px; } p { font-size: 12px; color: var(--text-dim); line-height: 1.5; }
button { min-height: 36px; padding: 6px 10px; font: inherit; font-size: 12px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-raised); color: var(--text); cursor: pointer; } button:disabled { opacity: .45; } button:focus-visible { outline: 2px solid var(--accent); }
.screen-choice { width: 100%; margin-top: 8px; text-align: left; display: flex; flex-direction: column; gap: 5px; } .screen-choice span { font-size: 10px; color: var(--text-dim); overflow-wrap: anywhere; } .screen-choice[aria-pressed=true] { border-color: var(--accent); } footer { margin-top: 14px; display: flex; gap: 8px; justify-content: flex-end; }
</style>
