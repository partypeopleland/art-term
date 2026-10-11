<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../transport/types'

const props = withDefaults(
  defineProps<{ host: RuntimeHostDto; transport: Transport; returnLabel?: string; initialGroup?: HostDto | null }>(),
  { returnLabel: '返回連線管理', initialGroup: null }
)
const emit = defineEmits<{ open: [host: HostDto, target: TargetDto]; close: [] }>()
// Optional seed: the caller's own just-fetched list for THIS frozen profile,
// avoiding a duplicate remote round-trip. Mismatched seeds never land; all
// generation/profile/visibility fences stay identical to a fresh load.
const group = ref<HostDto | null>(
  props.initialGroup && props.initialGroup.id === props.host.id ? props.initialGroup : null
)
const name = ref('')
const busy = ref(false)
const error = ref('')
let alive = true
let generation = 0
function current(request: number): boolean {
  return alive && request === generation && document.visibilityState !== 'hidden'
}
async function load(): Promise<void> {
  if (busy.value || !props.transport.runtimeSessions) return
  const request = ++generation
  busy.value = true
  error.value = ''
  try {
    const result = await props.transport.runtimeSessions(props.host.id, props.host.revision)
    if (current(request)) group.value = result
  } catch {
    if (current(request)) error.value = '無法列出 Shell，請確認連線與 runtime 版本。'
  } finally {
    if (current(request)) busy.value = false
  }
}
async function create(): Promise<void> {
  if (busy.value || !props.transport.runtimeCreateSession) return
  const selectedName = name.value.trim()
  if (!/^[a-z0-9-]{1,32}$/.test(selectedName)) {
    error.value = 'Shell 名稱請用 1–32 個小寫英文字母、數字或短橫線。'
    return
  }
  const request = ++generation
  busy.value = true
  error.value = ''
  try {
    const target = await props.transport.runtimeCreateSession(props.host.id, props.host.revision, selectedName)
    // desktopAvailable/codingCli come from the backend target DTO — the
    // explicit endpoint mapping, never the display name.
    if (current(request)) emit('open', { id: props.host.id, label: props.host.name, desktopAvailable: target.desktopAvailable, targets: [target] }, target)
  } catch {
    if (current(request)) error.value = '新增 Shell 未確認成功。請重新整理確認狀態，再決定是否重試。'
  } finally {
    if (current(request)) busy.value = false
  }
}
function visibility(): void {
  if (document.visibilityState === 'hidden') {
    generation++
    busy.value = false
    error.value = '若剛才建立 Shell 尚未確認，請先重新整理查看狀態。'
  }
}
onMounted(() => { document.addEventListener('visibilitychange', visibility); if (!group.value) void load() })
onBeforeUnmount(() => { alive = false; generation++; document.removeEventListener('visibilitychange', visibility) })
</script>

<template>
  <section class="runtime-sessions">
    <header><h3>{{ host.name }} 的 Shell</h3><button type="button" data-testid="sessions-return" @click="emit('close')">{{ returnLabel }}</button></header>
    <p v-if="error" role="alert" id="shell-error">{{ error }}</p>
    <div class="shell-heading"><h4>現有 Shell</h4><button type="button" :disabled="busy" @click="load">重新整理 Shell</button></div>
    <p v-if="busy" role="status">處理中…</p>
    <p v-else-if="group && !group.targets.length">尚無 Shell，可在下方建立新的 Shell。</p>
    <ul v-if="group">
      <li v-for="target in group.targets" :key="target.id">
        <div><strong>{{ target.label }}</strong><span class="shell-state">{{ target.phase === 'ready' ? '可連線' : target.phase === 'attached' ? '使用中' : target.phase === 'dead' ? '已結束' : target.phase === 'detached' ? '已脫離' : '狀態不可用' }}</span><small v-if="target.phase === 'attached'">接管會中斷同一 Shell 的原連線。</small></div>
        <button type="button" :disabled="busy || !target.running" @click="emit('open', group!, target)">{{ target.phase === 'attached' ? '接管連線' : '連線' }}</button>
      </li>
    </ul>
    <form @submit.prevent="create" :aria-describedby="error ? 'shell-error' : undefined">
      <h4>建立新 Shell</h4><p class="muted">關頁只脫離連線，遠端 Shell 繼續執行。</p>
      <label>新 Shell 名稱<input v-model="name" maxlength="32" autocomplete="off" autocapitalize="off" spellcheck="false" :disabled="busy" placeholder="例如 dev-shell"></label>
      <p class="muted">1–32 個小寫英文字母、數字或短橫線。</p>
      <button class="primary" type="submit" :disabled="busy">建立並連線</button>
    </form>
  </section>
</template>
<style scoped>
header, .shell-heading, li { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
h3 { font-size: 18px; margin: 0; } h4 { font-size: 14px; margin: 0; } .shell-heading { margin-top: 24px; }
ul { padding: 0; list-style: none; } li { padding: 14px 0; border-bottom: 1px solid var(--border, #465365); } li > div { min-width: 0; overflow-wrap: anywhere; }
.shell-state, small { display: block; margin-top: 5px; } .shell-state { color: var(--text-dim, #b3bfce); } small { color: var(--warn, #d7ba7d); line-height: 1.5; }
form { margin-top: 28px; padding-top: 20px; border-top: 1px solid var(--border, #465365); } label { display: flex; flex-direction: column; gap: 8px; margin-top: 16px; } input { width: 100%; background: var(--bg, #101722); padding: 10px; }
button, input { font: inherit; min-height: 44px; color: inherit; border: 1px solid var(--border, #6b7788); border-radius: 4px; } button { background: var(--bg-raised, #26282e); padding: 8px 12px; cursor: pointer; flex-shrink: 0; } button:hover:not(:disabled) { background: var(--bg-hover, #2e3038); } button:disabled { opacity: .6; cursor: wait; } .primary { border-color: var(--accent, #5b8def); }
:is(button, input):focus-visible { outline: 2px solid var(--accent, #5b8def); outline-offset: 3px; } .muted { color: var(--text-dim, #b3bfce); line-height: 1.6; } [role=alert] { color: #ffb7b7; }
@media (max-width: 480px) { header { align-items: flex-start; } h3 { font-size: 16px; } header button { max-width: 45%; } }
</style>
