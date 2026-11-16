<script setup lang="ts">
// Modal host picker for an empty split pane: reuses the same ConnectionDashboard
// as the main sidebar, so pane sessions are chosen/created through the exact
// same runtimeSessions + runtimeCreateHistorySession flow as top-level tabs.
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../transport/types'
import { useModalFocus } from './modal-focus'
import ConnectionDashboard from './ConnectionDashboard.vue'

const props = defineProps<{
  transport: Transport
  runtimeProfiles: RuntimeHostDto[]
  runtimeProfilesError: string
  catalogLoading: boolean
  runtimeConnected: Set<string>
}>()
const emit = defineEmits<{
  open: [host: HostDto, target: TargetDto]
  create: []
  close: []
}>()

const dialog = ref<HTMLElement | null>(null)
useModalFocus(dialog, () => emit('close'))
function visibility(): void {
  if (document.visibilityState === 'hidden') emit('close')
}
onMounted(() => document.addEventListener('visibilitychange', visibility))
onBeforeUnmount(() => document.removeEventListener('visibilitychange', visibility))
</script>

<template>
  <div class="pane-picker-backdrop" data-testid="pane-connection-picker">
    <section ref="dialog" tabindex="-1" class="pane-picker" role="dialog" aria-modal="true" aria-label="為窗格選擇連線">
      <header class="pane-picker-head">
        <h2>為窗格選擇連線</h2>
        <button type="button" class="btn" @click="emit('close')">關閉</button>
      </header>
      <div class="pane-picker-body">
        <ConnectionDashboard
          :transport="transport"
          :active="true"
          :catalog-loading="catalogLoading"
          :runtime-profiles="runtimeProfiles"
          :runtime-profiles-error="runtimeProfilesError"
          :runtime-pending="null"
          :runtime-connected="runtimeConnected"
          @open="(host, target) => emit('open', host, target)"
          @create="emit('create')"
        />
      </div>
    </section>
  </div>
</template>

<style scoped>
.pane-picker-backdrop { position: fixed; inset: 0; z-index: 100; background: #0008; display: flex; justify-content: center; align-items: center; padding: 16px; }
.pane-picker { background: var(--bg-panel, #18202b); color: var(--text, #e8edf5); width: min(560px, 100%); max-height: calc(100dvh - 32px); display: flex; flex-direction: column; border: 1px solid #6b7788; border-radius: 8px; overflow: hidden; }
.pane-picker-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 18px; border-bottom: 1px solid var(--border); }
.pane-picker-head h2 { font-size: 16px; margin: 0; }
.pane-picker-body { overflow: auto; padding: 8px 14px 16px; }
</style>
