<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import type { HostDto, TargetDto, RuntimeHostDto, RuntimeHostInput, RuntimeHostTestDto, Transport } from '../transport/types'
import { hostErrorText } from '../transport/host-error'
import { useModalFocus } from './modal-focus'
import RuntimeSessions from './RuntimeSessions.vue'
import RuntimeTransfer from './RuntimeTransfer.vue'
import { getTestComputerPrefill } from '../test-computer-prefill'

const props = withDefaults(defineProps<{ transport: Transport; initialView?: 'list' | 'create'; initialCategory?: 'host' | 'agent'; categoryFilter?: 'host' | 'agent'; embedded?: boolean }>(), { initialView: 'list' })
const view = ref<'list' | 'form'>(props.initialView === 'create' ? 'form' : 'list')
const category = ref<'host' | 'agent'>(props.initialCategory ?? 'host')
const dialog = ref<HTMLElement | null>(null)
if (!props.embedded) useModalFocus(dialog, close)
const emit = defineEmits<{ close: []; changed: []; picker: [active: boolean]; open: [host: HostDto, target: TargetDto] }>()
const transferMode = ref<'import' | 'export' | null>(null)
const sessionHost = ref<RuntimeHostDto | null>(null)
const hosts = ref<RuntimeHostDto[]>([])
const visibleHosts = computed(() => hosts.value.filter(host => !props.categoryFilter || (host.category === 'agent' ? 'agent' : 'host') === props.categoryFilter))
const editing = ref<RuntimeHostDto | null>(null)
const name = ref('')
const baseUrl = ref('')
const credential = ref('')
const testPrefillApplied = ref(false)
const formTouched = ref(false)
const credentialInput = ref<HTMLInputElement | null>(null)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const result = ref<{ id: string; value: RuntimeHostTestDto } | null>(null)
const deleting = ref<RuntimeHostDto | null>(null)
// Every transport in the runtime-only product implements the full
// runtime Host surface; the flag only gates rendering.
const available = computed(() => true)
let generation = 0
let alive = true

function clearCredential(): void {
  testPrefillApplied.value = false
  credential.value = ''
  if (credentialInput.value) credentialInput.value.value = ''
}
function clearPending(): void {
  generation++
  clearCredential()
  busy.value = false
}
function current(request: number): boolean {
  return alive && request === generation && document.visibilityState !== 'hidden'
}
function reset(): void {
  clearPending()
  formTouched.value = false
  editing.value = null
  deleting.value = null
  category.value = props.initialCategory ?? 'host'
  name.value = ''
  baseUrl.value = ''
  error.value = ''
}
function create(): void { reset(); view.value = 'form'; notice.value = ''; void nextTick(() => dialog.value?.querySelector<HTMLInputElement>('[data-testid=name]')?.focus()) }
function cancel(): void { if (props.embedded) { reset(); view.value = 'form'; return } reset(); view.value = props.embedded ? 'form' : 'list'; void nextTick(() => dialog.value?.querySelector<HTMLButtonElement>('[data-testid=runtime-create]')?.focus()) }
function edit(host: RuntimeHostDto): void {
  reset()
  view.value = 'form'
  category.value = host.category === 'agent' ? 'agent' : 'host'
  editing.value = { ...host }
  void nextTick(() => dialog.value?.querySelector<HTMLInputElement>('[data-testid=name]')?.focus())
  name.value = host.name
  baseUrl.value = host.baseUrl
  notice.value = ''
}
function endpoint(value: string): string | null {
  if (value.length > 2048 || /[\\\u0000-\u001f\u007f]/.test(value)) return null
  // Validate raw authority/path first: URL parsing would normalize dot paths,
  // backslashes and controls into a different endpoint.
  const raw = value.trim()
  const shape = /^https:\/\/([^/?#]+)\/?$/i.exec(raw)
  if (!shape || shape[1].includes('@')) return null
  try {
    const url = new URL(raw)
    if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash || url.pathname !== '/') return null
    return url.origin
  } catch {
    return null
  }
}
async function reload(): Promise<void> {
  if (!available.value) return
  clearPending()
  const request = generation
  busy.value = true
  error.value = ''
  try {
    const values = await props.transport.runtimeHosts!()
    if (current(request)) {
      if (values.length > 64) throw new Error('bounded')
      hosts.value = values
    }
  } catch (e) {
    if (current(request)) error.value = hostErrorText(e, '無法讀取 Host 設定，請關閉後重新開啟。')
  } finally {
    // Even stale completion must not leave a subsequently typed secret alive.
    clearCredential()
    if (current(request)) busy.value = false
  }
}
async function save(): Promise<void> {
  if (busy.value || !available.value) return
  let secret = credential.value
  clearCredential()
  error.value = ''
  const canonical = endpoint(baseUrl.value)
  if (!name.value.trim() || new TextEncoder().encode(name.value.trim()).length > 128 || !canonical) {
    error.value = '請填寫 1–128 UTF-8 位元組名稱及 HTTPS 根網址；不可包含帳密、路徑、查詢或片段。'
    return
  }
  if (new TextEncoder().encode(secret).length > 1024 || (!secret && (!editing.value || endpoint(editing.value.baseUrl) !== canonical))) {
    error.value = '新增或變更網址時須輸入新的管理憑證（最多 1024 UTF-8 位元組）。'
    return
  }
  if (!editing.value && hosts.value.length >= 64) {
    error.value = 'Host 數量已達上限（64）。'
    return
  }
  const request = ++generation
  busy.value = true
  try {
    const pending = props.transport.runtimeSaveHost!({
      name: name.value.trim(), baseUrl: canonical, credential: secret, category: category.value,
      ...(editing.value ? { id: editing.value.id, expectedRevision: editing.value.revision } : {})
    } satisfies RuntimeHostInput)
    // The transport owns its one-shot argument; release our copy before awaiting.
    secret = ''
    const saved = await pending
    if (alive) emit('changed')
    if (current(request)) {
      hosts.value = [...hosts.value.filter((host) => host.id !== saved.id), saved]
      cancel()
      notice.value = '設定已儲存。可測試已儲存的 Host。'
      result.value = null
    }
  } catch (e) {
    if (current(request)) error.value = hostErrorText(e, '無法儲存 Host。請確認網址與憑證，或關閉後重新開啟再試。')
  } finally {
    secret = ''
    clearCredential()
    if (current(request)) busy.value = false
  }
}
async function remove(): Promise<void> {
  if (busy.value || !deleting.value || !available.value) return
  clearCredential()
  const host = deleting.value
  const request = ++generation
  busy.value = true
  error.value = ''
  try {
    await props.transport.runtimeDeleteHost!(host.id, host.revision)
    if (alive) emit('changed')
    if (current(request)) {
      hosts.value = hosts.value.filter((item) => item.id !== host.id)
      reset()
      notice.value = '本機 Host 設定已刪除；遠端 Shell 仍保留。'
      result.value = null
    }
  } catch (e) {
    if (current(request)) error.value = hostErrorText(e, '無法刪除 Host 設定，請關閉後重新開啟再試。')
  } finally {
    clearCredential()
    if (current(request)) busy.value = false
  }
}
async function test(host: RuntimeHostDto): Promise<void> {
  if (busy.value || !available.value) return
  clearCredential()
  const request = ++generation
  busy.value = true
  error.value = ''
  result.value = null
  try {
    const value = await props.transport.runtimeTestHost!(host.id, host.revision)
    if (current(request)) result.value = { id: host.id, value }
  } catch (e) {
    if (current(request)) error.value = hostErrorText(e, 'Host 測試失敗。請確認連線、TLS 憑證及管理憑證。')
  } finally {
    clearCredential()
    if (current(request)) busy.value = false
  }
}
function visibility(): void {
  if (document.visibilityState === 'hidden') clearPending()
}
function close(): void {
  reset()
  emit('close')
}
onMounted(async () => {
  document.addEventListener('visibilitychange', visibility)
  const initialRequest = generation + 1
  await reload()
  if (!current(initialRequest) || formTouched.value || editing.value || name.value || baseUrl.value || credential.value) return
  const prefill = getTestComputerPrefill()
  if (!prefill || props.initialView !== 'create') return
  name.value = prefill.name
  baseUrl.value = prefill.baseUrl
  credential.value = prefill.credential
  testPrefillApplied.value = true
})
onBeforeUnmount(() => {
  alive = false
  clearPending()
  document.removeEventListener('visibilitychange', visibility)
})
</script>

<template>
  <div class="runtime-host-backdrop" :class="{ 'embedded-host-page': embedded }">
    <section ref="dialog" tabindex="-1" class="runtime-hosts" :role="embedded ? 'region' : 'dialog'" :aria-modal="embedded ? undefined : true" aria-labelledby="runtime-host-heading">
      <header><h2 id="runtime-host-heading">{{ view === 'list' ? '管理 openab-pty 連線' : editing ? '編輯 openab-pty 連線' : '新增 openab-pty 連線' }}</h2><button v-if="!embedded" type="button" @click="close">關閉</button></header>
      <RuntimeTransfer v-if="transferMode" :mode="transferMode" :transport="transport" :hosts="hosts" @close="transferMode = null; void reload()" @changed="emit('changed'); void reload()" @picker="emit('picker', $event)" />
      <RuntimeSessions v-else-if="sessionHost" :host="sessionHost" :transport="transport" @close="sessionHost = null" @open="(host, target) => emit('open', host, target)" />
      <template v-else>
      <p v-if="!available">此版本尚未提供 Runtime Host 設定。</p>
      <template v-else>
        <p v-if="view === 'form'">請填寫已部署的 openab-pty 服務 HTTPS 根網址與 Bearer auth token。先儲存設定，再測試連線。</p>
        <p v-if="error" role="alert" id="runtime-host-error">{{ error }}</p>
        <p v-if="notice" role="status">{{ notice }}</p>
        <p v-if="testPrefillApplied" role="status">私人測試預填：尚未儲存，請手動儲存後再測試。</p>
        <div v-if="view === 'list' || embedded" data-testid="runtime-host-list">
        <div v-if="!embedded" class="transfer-actions"><button type="button" :disabled="busy" data-testid="runtime-import" @click="reset(); transferMode = 'import'">匯入連線</button><button type="button" :disabled="busy || !visibleHosts.length" data-testid="runtime-export" @click="reset(); transferMode = 'export'">匯出連線</button></div>
        <button v-if="!embedded" type="button" data-testid="runtime-create" :disabled="busy" @click="create">新增 openab-pty 連線</button>
        <p v-if="busy" role="status">處理中…</p>
        <p v-else-if="!visibleHosts.length">尚未儲存連線。選擇新增連線開始設定。</p>
        <ul>
          <li v-for="host in visibleHosts" :key="host.id" :data-host-id="host.id">
            <strong>{{ host.name }}</strong> <span>{{ host.category === 'agent' ? 'Agent' : 'Host' }}</span> <span>{{ host.baseUrl }}</span>
            <span>{{ host.credentialPresent ? '管理憑證已儲存' : '缺少管理憑證' }}</span>
            <button type="button" :disabled="busy" @click="edit(host)">編輯</button>
            <button type="button" :disabled="busy" @click="test(host)">測試</button>
            <button v-if="!embedded && transport.runtimeSessions && transport.runtimeCreateSession" type="button" :disabled="busy" @click="clearCredential(); sessionHost = host">Shell</button>
            <button type="button" :disabled="busy" @click="reset(); deleting = host">刪除</button>
            <p v-if="result?.id === host.id" role="status">
              已通過管理認證；{{ result.value.sessionCount }} 個 Shell；
              {{ result.value.draining ? '停止接受新 Shell' : '可接受新 Shell' }}；
              {{ result.value.teardownBestEffort ? '清理為盡力而為' : '未標記盡力清理' }}；
              {{ result.value.continuityVerified ? '輸出連續性已驗證' : '輸出連續性未驗證' }}。
            </p>
          </li>
        </ul>
        <div v-if="deleting" class="delete-confirm">
          <p>刪除「{{ deleting.name }}」的本機設定？現有遠端 Shell 不會被終止。</p>
          <button type="button" :disabled="busy" data-testid="confirm-delete" @click="remove">確認刪除本機設定</button>
          <button type="button" @click="reset">取消</button>
        </div>
        </div>
        <form v-if="view === 'form' && !deleting" data-testid="runtime-host-form" @submit.prevent="save" :aria-describedby="error ? 'runtime-host-error' : undefined">

          <label>分類<select v-model="category" data-testid="runtime-host-category" :disabled="busy" @change="formTouched = true"><option value="host">Host</option><option value="agent">Agent</option></select></label>
          <label>名稱<input v-model="name" @input="formTouched = true" data-testid="name" :disabled="busy" maxlength="128" autocomplete="off" /></label>
          <label>openab-pty 服務網址<input v-model="baseUrl" @input="formTouched = true" data-testid="base-url" :disabled="busy" maxlength="2048" type="url" placeholder="https://runtime.example.test:8443" autocomplete="off" /></label>
          <label>管理憑證（Bearer auth token）<input ref="credentialInput" v-model="credential" @input="formTouched = true" data-testid="credential" :disabled="busy" type="password" maxlength="1024" autocomplete="new-password" autocapitalize="off" spellcheck="false" /></label>
          <p v-if="editing">憑證留空可保留原值；變更網址時須重新輸入。憑證提交後立即清除。</p>
          <button type="submit" :disabled="busy">{{ busy ? '處理中…' : '儲存' }}</button>
          <button type="button" data-testid="runtime-form-cancel" @click="cancel">取消</button>
        </form>
      </template>
      </template>
    </section>
  </div>
</template>

<style scoped>
.runtime-host-backdrop { position: fixed; inset: 0; z-index: 100; background: #0008; display: flex; justify-content: center; align-items: center; padding: 16px; }
.runtime-hosts { background: var(--bg-panel, #18202b); color: var(--text, #e8edf5); width: min(680px, 100%); max-height: calc(100dvh - 32px); overflow: auto; padding: 20px; border: 1px solid #6b7788; border-radius: 8px; }
header { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
label { display: flex; flex-direction: column; gap: 5px; margin: 12px 0; }
input, select { min-height: 44px; font: inherit; padding: 9px; background: #101722; color: inherit; border: 1px solid #6b7788; border-radius: 4px; }
button { min-height: 44px; font: inherit; background: var(--bg-raised, #26282e); color: inherit; border: 1px solid #6b7788; border-radius: 4px; padding: 7px 10px; margin: 4px; cursor: pointer; }
button:disabled { cursor: wait; opacity: .6; }
ul { padding: 0; list-style: none; }
li { padding: 12px 0; border-bottom: 1px solid #465365; overflow-wrap: anywhere; }
li > span { display: block; margin: 5px 0; }
[role="alert"] { color: #ffb7b7; }
h2 { font-size: 18px; margin: 0; } p { line-height: 1.6; }
button:hover:not(:disabled) { background: var(--bg-hover, #2e3038); }
:is(button, input, select):focus-visible { outline: 2px solid var(--accent, #5b8def); outline-offset: 3px; }
@media (max-width: 480px) { .runtime-host-backdrop { padding: 8px; } .runtime-hosts { padding: 16px; max-height: calc(100dvh - 16px); } }
</style>

<style scoped>
.runtime-host-backdrop.embedded-host-page.embedded-host-page { position: static; display: block; padding: 14px; background: transparent; }
.embedded-host-page.embedded-host-page .runtime-hosts { width: 100%; max-width: 600px; max-height: none; margin: 0 auto; padding: 16px; overflow: visible; background: var(--bg-panel); border: 1px solid var(--border); border-radius: 12px; }
.embedded-host-page input, .embedded-host-page select { background: var(--bg); border-color: var(--border); }
.embedded-host-page button { border-color: var(--border); }
.embedded-host-page ul { list-style: none; padding: 0; margin: 14px 0; }
.embedded-host-page li { display: flex; flex-wrap: wrap; align-items: center; gap: 7px; padding: 12px 0; border-bottom: 1px solid var(--border); font-size: 11px; }
.embedded-host-page li > strong { flex-basis: 100%; font-size: 13px; }
.embedded-host-page li > span { color: var(--text-dim); overflow-wrap: anywhere; }
.embedded-host-page li > button { min-height: 32px; padding: 4px 9px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-raised); color: var(--text); }
.embedded-host-page li > p { flex-basis: 100%; }
</style>
