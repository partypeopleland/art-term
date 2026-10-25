<script setup lang="ts">
// Shared modal (Windows + Android) for a saved runtime connection picked from
// the outer connection list. The parent freezes the profile snapshot (id +
// revision); the key remounts the session panel on reselection, and
// RuntimeSessions' own generation/visibility guards discard any late
// completion. Session listing starts only on mount — after the explicit
// click — and nothing creates/attaches/takes over without the existing
// explicit buttons. Backgrounding closes the dialog and drops the
// selection; existing terminal tabs are unaffected.
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../transport/types'
import { useModalFocus } from './modal-focus'
import RuntimeSessions from './RuntimeSessions.vue'

withDefaults(
  defineProps<{ profile: RuntimeHostDto; transport: Transport; initialGroup?: HostDto | null }>(),
  { initialGroup: null }
)
const emit = defineEmits<{ open: [host: HostDto, target: TargetDto]; close: [] }>()

const dialog = ref<HTMLElement | null>(null)
useModalFocus(dialog, () => emit('close'))
function visibility(): void {
  if (document.visibilityState === 'hidden') emit('close')
}
onMounted(() => document.addEventListener('visibilitychange', visibility))
onBeforeUnmount(() => document.removeEventListener('visibilitychange', visibility))
</script>

<template>
  <div class="runtime-conn-backdrop" data-testid="runtime-connection-dialog">
    <section ref="dialog" tabindex="-1" class="runtime-conn" role="dialog" aria-modal="true" :aria-label="profile.name">
      <RuntimeSessions
        :key="`${profile.id}:${profile.revision}`"
        :host="profile"
        :transport="transport"
        :initial-group="initialGroup"
        return-label="返回連線清單"
        @open="(host, target) => emit('open', host, target)"
        @close="emit('close')"
      />
    </section>
  </div>
</template>

<style scoped>
.runtime-conn-backdrop { position: fixed; inset: 0; z-index: 100; background: #0008; display: flex; justify-content: center; align-items: center; padding: 16px; }
.runtime-conn { background: var(--bg-panel, #18202b); color: var(--text, #e8edf5); width: min(680px, 100%); max-height: calc(100dvh - 32px); overflow: auto; padding: 20px; border: 1px solid #6b7788; border-radius: 8px; }
</style>
