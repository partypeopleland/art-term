<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../transport/types'

const props = defineProps<{
  profile: RuntimeHostDto
  transport: Transport
  connected: boolean
  active: boolean
}>()
const emit = defineEmits<{ open: [host: HostDto, target: TargetDto] }>()
const group = ref<HostDto | null>(null)
const busy = ref(false)
const error = ref('')
const expanded = ref(false)
const creating = ref(false)
let generation = 0
let alive = true
function current(ticket: number, profile: RuntimeHostDto): boolean {
  return alive && props.active && !document.hidden && ticket === generation && props.profile.id === profile.id && props.profile.revision === profile.revision
}
async function load(): Promise<void> {
  if (busy.value || !props.active || document.hidden) return
  const profile = { ...props.profile }
  const ticket = ++generation
  busy.value = true
  error.value = ''
  try {
    const result = await props.transport.runtimeSessions(profile.id, profile.revision)
    if (current(ticket, profile)) {
      if (result.id === profile.id) group.value = result
      else error.value = '連線設定已變更，請重新整理。'
    }
  } catch {
    if (current(ticket, profile)) error.value = '無法讀取連線，點主機名稱重試。'
  } finally {
    if (current(ticket, profile)) busy.value = false
  }
}
function attach(target: TargetDto): void {
  if (!busy.value && group.value && target.running && props.active && !document.hidden) emit('open', group.value, target)
}
async function create(): Promise<void> {
  if (busy.value || creating.value || !props.active || document.hidden || !props.transport.runtimeCreateHistorySession) return
  const profile = { ...props.profile }
  const ticket = ++generation
  busy.value = true
  creating.value = true
  error.value = ''
  try {
    const target = await props.transport.runtimeCreateHistorySession(profile.id, profile.revision)
    if (current(ticket, profile)) emit('open', { id: profile.id, label: profile.name, desktopAvailable: target.desktopAvailable, targets: [target] }, target)
  } catch {
    if (current(ticket, profile)) error.value = '建立結果未確認，請先重新整理連線。'
  } finally {
    creating.value = false
    if (current(ticket, profile)) busy.value = false
  }
}
function cancelPending(): void { generation++; busy.value = false }
function visibility(): void {
  if (document.hidden) cancelPending()
  else if (props.active) void load()
}
watch(() => props.profile, () => { cancelPending(); group.value = null; expanded.value = false; void load() })
watch(() => props.active, active => { if (active) void load(); else cancelPending() })
onMounted(() => { document.addEventListener('visibilitychange', visibility); void load() })
onBeforeUnmount(() => { alive = false; cancelPending(); document.removeEventListener('visibilitychange', visibility) })
</script>

<template>
  <div class="host-connections" :data-testid="`runtime-${profile.id}`">
    <div class="host-connection-grid">
      <button class="host-identity" :data-runtime-label="profile.name" :disabled="busy" :aria-label="`${profile.agentName || profile.name}，${(group?.targets.length ?? 0) > 1 ? expanded ? '收合連線' : '展開連線' : '重新整理連線'}`" :aria-expanded="(group?.targets.length ?? 0) > 1 ? expanded : undefined" @click="(group?.targets.length ?? 0) > 1 ? expanded = !expanded : load()">
        <span class="connection-dot" :class="{ on: connected }" :data-testid="`runtime-status-${profile.id}`" :title="connected ? '已連線（本機分頁）' : '未連線'" />
        <span class="host-copy"><strong>{{ profile.agentName || profile.name }}</strong><small>{{ profile.agentTool || 'openab-pty' }}</small></span>
      </button>
      <div class="existing-connections" aria-label="既有連線">
        <span v-if="busy && !group" class="session-empty">讀取中…</span>
        <span v-else-if="error && !group?.targets.length" class="session-empty">讀取失敗</span>
        <div v-if="(group?.targets.length ?? 0) > 1" class="connection-summary-row">
          <button v-if="!expanded" class="attach-button connection-summary" :disabled="busy" :aria-expanded="false" @click="expanded = true"><span>連線數:{{ String(group?.targets.length ?? 0).padStart(2, '0') }}</span></button>
          <button v-else class="attach-button" :disabled="busy || !group!.targets[0].running" :title="group!.targets[0].phase === 'attached' ? '接管此連線，會中斷原連線' : 'Attach 此連線'" @click="attach(group!.targets[0])"><span>{{ group!.targets[0].label }}</span><small v-if="group!.targets[0].phase === 'attached'">接管</small><span v-else class="attach-arrow" aria-hidden="true">↗</span></button>
        </div>
        <template v-if="expanded || (group?.targets.length ?? 0) <= 1">
          <div v-for="target in ((group?.targets.length ?? 0) > 1 ? group!.targets.slice(1) : group?.targets ?? [])" :key="target.id" class="connection-summary-row">
            <button class="attach-button" :disabled="busy || !target.running" :title="target.phase === 'attached' ? '接管此連線，會中斷原連線' : 'Attach 此連線'" @click="attach(target)"><span>{{ target.label }}</span><small v-if="target.phase === 'attached'">接管</small><span v-else class="attach-arrow" aria-hidden="true">↗</span></button>
          </div>
        </template>
      </div>

      <button class="new-connection" :disabled="busy || creating || !transport.runtimeCreateHistorySession" aria-label="新連線" title="新連線" @click="create"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg></button>
    </div>
    <p v-if="error" class="connection-error" role="alert">{{ error }}</p>
  </div>
</template>

<style scoped>
.host-connections { border-top: 1px solid var(--border); padding: 5px 0; }
.host-connection-grid { display: grid; grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr) 32px; gap: 6px; align-items: start; }
button { font: inherit; cursor: pointer; touch-action: manipulation; }
button:focus-visible { outline: 2px solid var(--section-accent); outline-offset: -2px; }
button:disabled { opacity: .5; cursor: default; }
.host-identity { display: flex; align-items: center; gap: 8px; min-width: 0; min-height: 30px; padding: 3px 2px; border: 0; border-radius: 5px; background: transparent; color: var(--text); text-align: left; }
.host-copy { display: flex; flex-direction: column; min-width: 0; gap: 3px; }
.host-copy strong { font-size: 11px; font-weight: 550; overflow-wrap: anywhere; line-height: 1.4; }
.host-copy small { font-size: 9px; color: var(--text-dim); }
.connection-dot { flex: none; width: 6px; height: 6px; border-radius: 50%; background: var(--dot-idle); margin-left: 2px; }
.connection-dot.on { background: var(--ok); box-shadow: 0 0 7px color-mix(in srgb, var(--ok) 35%, transparent); }
.existing-connections { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
.attach-button { display: flex; align-items: center; justify-content: space-between; gap: 4px; width: 100%; min-height: 30px; padding: 3px 7px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); color: var(--section-accent); text-align: left; font-size: 11px; }
.attach-button > span:first-child { overflow-wrap: anywhere; min-width: 0; }
.attach-button:hover { background: var(--bg-raised); border-color: var(--section-accent); }
.attach-button small { flex: none; font-size: 9px; color: var(--warn); }
.attach-arrow { color: var(--text-dim); font-size: 11px; }
.session-empty { display: flex; align-items: center; min-height: 30px; padding: 0 4px; color: var(--text-dim); font-size: 10px; }
.new-connection { display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 2px; width: 32px; min-height: 30px; padding: 3px 2px; border: 1px solid var(--border); border-radius: 6px; color: var(--section-accent); background: var(--bg-raised); font-size: 10px; }
.new-connection > span:first-child { font-size: 15px; line-height: 1; }
.new-connection:hover { border-color: var(--section-accent); }
.connection-error { margin: 4px 2px 0; font-size: 10px; line-height: 1.5; color: var(--err); }

.host-connection-grid { align-items: start; }
.host-identity { padding-block: 1px; }
.host-copy { gap: 1px; }
.attach-button { font-size: 10px; }
.new-connection { height: 30px; padding: 6px; }
.new-connection svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; }
.collapse-connections { min-height: 22px; padding: 2px 5px; border: 0; border-radius: 4px; background: transparent; color: var(--text-dim); font-size: 9px; text-align: right; }

.connection-summary { justify-content: flex-start; gap: 5px; }
.expand-chevron { flex: none; width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; }
.expand-chevron.expanded { transform: rotate(90deg); }

.connection-summary-row { display: flex; align-items: center; gap: 3px; min-width: 0; }
.expand-toggle { display: grid; place-items: center; flex: none; width: 16px; min-height: 30px; padding: 0; border: 0; background: transparent; color: var(--section-accent); cursor: pointer; }
.connection-summary-row .connection-summary { flex: 1; min-width: 0; padding-inline: 5px; font-size: 10px; white-space: nowrap; }

.connection-summary-row .attach-button { flex: 1; width: 0; min-width: 0; }
.expand-spacer { flex: none; width: 16px; }

.host-identity:hover { background: rgba(var(--section-rgb), .04); }

.host-connection-grid { grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr) 32px; }
</style>
