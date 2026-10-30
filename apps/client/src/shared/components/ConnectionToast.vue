<script setup lang="ts">
import { onBeforeUnmount, watch } from 'vue'
const props = defineProps<{ notification: { message: string; error: boolean } }>()
const emit = defineEmits<{ dismiss: [] }>()
let timer: ReturnType<typeof setTimeout> | undefined
watch(() => props.notification, value => {
  clearTimeout(timer)
  timer = setTimeout(() => emit('dismiss'), value.error ? 8000 : 3000)
}, { immediate: true })
onBeforeUnmount(() => clearTimeout(timer))
</script>
<template>
  <div class="connection-toast" :class="{ failure: notification.error }" data-testid="connection-toast" :role="notification.error ? 'alert' : 'status'" aria-atomic="true"><span>{{ notification.message }}</span><button type="button" aria-label="關閉提示" @click="emit('dismiss')">×</button></div>
</template>
<style scoped>
.connection-toast { position: fixed; z-index: 1000; top: max(24px, env(safe-area-inset-top)); left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 16px; max-width: min(560px, calc(100vw - 32px)); box-sizing: border-box; padding: 10px 12px 10px 18px; border: 1px solid var(--ok, #65c499); border-radius: 10px; color: var(--text); background: var(--bg-panel, #18202b); box-shadow: 0 6px 24px #0005; font-size: 13px; line-height: 1.5; overflow-wrap: anywhere; } .connection-toast.failure { border-color: var(--err); color: var(--err); } .connection-toast button { flex: none; width: 32px; min-height: 32px; padding: 0; background: transparent; border: 0; font-size: 22px; }
button { font: inherit; color: inherit; cursor: pointer; } button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
