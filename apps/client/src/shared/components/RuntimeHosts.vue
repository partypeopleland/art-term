<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import type { HostDto, TargetDto, RuntimeHostDto, RuntimeHostInput, InstanceDesktop, Transport } from '../transport/types'
import { hostErrorText } from '../transport/host-error'
import { useModalFocus } from './modal-focus'
import RuntimeTransfer from './RuntimeTransfer.vue'
import ConnectionIcon from './ConnectionIcon.vue'
import ConnectionToast from './ConnectionToast.vue'
import { getTestComputerPrefill } from '../test-computer-prefill'

type Kind = 'host' | 'agent' | 'screen'
type Row = { kind: 'host' | 'agent'; value: RuntimeHostDto } | { kind: 'screen'; value: InstanceDesktop }
type Drag = { pointer: number; row: Row; overId: string | null; after: boolean }
const props = withDefaults(defineProps<{ transport: Transport; initialView?: 'list' | 'create'; initialCategory?: 'host' | 'agent'; categoryFilter?: 'host' | 'agent'; embedded?: boolean }>(), { initialView: 'list' })
const emit = defineEmits<{ close: []; changed: []; picker: [active: boolean]; open: [host: HostDto, target: TargetDto] }>()
const view = ref<'list' | 'form'>(props.initialView === 'create' ? 'form' : 'list')
const category = ref<Kind>(props.initialCategory ?? 'host')
const dialog = ref<HTMLElement | null>(null)
if (!props.embedded) useModalFocus(dialog, close)
const transferMode = ref<'import' | 'export' | null>(null)
const hosts = ref<RuntimeHostDto[]>([]), screens = ref<InstanceDesktop[]>([])
const editing = ref<Row | null>(null), deleting = ref<Row | null>(null)
const name = ref(''), baseUrl = ref(''), credential = ref(''), displayName = ref('')
const testPrefillApplied = ref(false), formTouched = ref(false), busy = ref(false), error = ref('')
const credentialInput = ref<HTMLInputElement | null>(null)
const toast = ref<{ message: string; error: boolean } | null>(null), drag = ref<Drag | null>(null)
let generation = 0, alive = true
let previousKind: Kind = props.initialCategory ?? 'host'
const kinds: { kind: Kind; title: string }[] = [{ kind: 'host', title: 'Computer' }, { kind: 'agent', title: 'Agent' }, { kind: 'screen', title: 'Screen' }]
const rows = computed<Row[]>(() => [
  ...hosts.value.map(value => ({ kind: value.category === 'agent' ? 'agent' as const : 'host' as const, value })),
  ...screens.value.map(value => ({ kind: 'screen' as const, value }))
])
const groups = computed(() => kinds.filter(item => !props.categoryFilter || item.kind === props.categoryFilter).map(item => ({ ...item, rows: rows.value.filter(row => row.kind === item.kind) })))
function rowUrl(row: Row): string { return row.kind === 'screen' ? row.value.mcpUrl : row.value.baseUrl }
function dismissToast(): void { toast.value = null }
function notify(message: string, failure = false): void { toast.value = { message, error: failure } }
function clearCredential(): void { testPrefillApplied.value = false; credential.value = ''; if (credentialInput.value) credentialInput.value.value = '' }
function clearPending(): void { generation++; clearCredential(); busy.value = false; drag.value = null }
function current(request: number): boolean { return alive && request === generation && document.visibilityState !== 'hidden' }
function reset(): void {
  clearPending(); formTouched.value = false; editing.value = null; deleting.value = null
  category.value = props.initialCategory ?? 'host'; previousKind = category.value; name.value = ''; baseUrl.value = ''; displayName.value = ''; error.value = ''
}
function create(): void { reset(); view.value = 'form'; void nextTick(() => dialog.value?.querySelector<HTMLInputElement>('[data-testid=name]')?.focus()) }
function cancel(): void { reset(); view.value = 'list'; void nextTick(() => dialog.value?.querySelector<HTMLButtonElement>('[data-testid=runtime-create]')?.focus()) }
function edit(row: Row): void {
  reset(); view.value = 'form'; editing.value = row; category.value = row.kind; previousKind = row.kind; name.value = row.value.name; baseUrl.value = rowUrl(row)
  displayName.value = row.kind === 'screen' ? '' : row.value.displayName ?? ''
  void nextTick(() => dialog.value?.querySelector<HTMLInputElement>('[data-testid=name]')?.focus())
}
function changeKind(): void { formTouched.value = true; if (category.value === 'screen' || previousKind === 'screen') clearCredential(); previousKind = category.value; error.value = '' }
function endpoint(value: string): string | null {
  if (value.length > 2048 || /[\\\u0000-\u001f\u007f]/.test(value)) return null
  const shape = /^https:\/\/([^/?#]+)\/?$/i.exec(value.trim())
  if (!shape || shape[1]?.includes('@')) return null
  try { const url = new URL(value.trim()); return url.protocol === 'https:' && !url.username && !url.password && !url.search && !url.hash && url.pathname === '/' ? url.origin : null } catch { return null }
}
async function reload(): Promise<void> {
  clearPending(); const request = generation; busy.value = true
  try {
    const [savedHosts, savedScreens] = await Promise.all([props.transport.runtimeHosts(), props.transport.instanceDesktops?.() ?? []])
    if (current(request)) { if (savedHosts.length > 64 || savedScreens.length > 64) throw new Error('bounded'); hosts.value = savedHosts; screens.value = savedScreens }
  } catch (e) { if (current(request)) notify(hostErrorText(e, '無法讀取連線設定，請關閉後重新開啟。'), true) }
  finally { clearCredential(); if (current(request)) busy.value = false }
}
async function save(): Promise<void> {
  if (busy.value) return
  let secret = credential.value; clearCredential(); error.value = ''
  const bytes = new TextEncoder()
  if (!name.value.trim() || bytes.encode(name.value.trim()).length > 128) { error.value = '請填寫 1–128 UTF-8 位元組名稱。'; return }
  if (category.value !== 'screen' && ([...displayName.value.trim()].length > 20 || /[\u0000-\u001f\u007f]/.test(displayName.value))) { error.value = '顯示名稱最多 20 個字元，不可包含控制字元。'; return }
  const canonical = category.value === 'screen' ? baseUrl.value.trim() : endpoint(baseUrl.value)
  if (!canonical) { error.value = '請填寫 HTTPS 根網址；不可包含帳密、路徑、查詢或片段。'; return }
  const sameUrl = editing.value && (category.value === 'screen' ? rowUrl(editing.value) === canonical : endpoint(rowUrl(editing.value)) === canonical)
  if (bytes.encode(secret).length > 1024 || (!secret && !sameUrl)) { error.value = '新增或變更網址時須輸入新的 Token（最多 1024 UTF-8 位元組）。'; return }
  if (!editing.value && (category.value === 'screen' ? screens.value : hosts.value).length >= 64) { error.value = '此類連線數量已達上限（64）。'; return }
  const request = ++generation; busy.value = true
  try {
    const identity = editing.value ? { id: editing.value.value.id, expectedRevision: editing.value.value.revision } : {}
    if (category.value === 'screen') {
      if (!props.transport.instanceSave) throw new Error('unavailable')
      const input = { ...identity, name: name.value.trim(), mcpUrl: canonical, token: secret }
      const pending = props.transport.instanceSave(input); secret = ''
      const saved = await pending
      if (current(request)) { const index = screens.value.findIndex(item => item.id === saved.id); if (index < 0) screens.value.push(saved); else screens.value.splice(index, 1, saved) }
    } else {
      const input: RuntimeHostInput = { ...identity, name: name.value.trim(), baseUrl: canonical, credential: secret, category: category.value, displayName: displayName.value.trim() }
      const pending = props.transport.runtimeSaveHost(input); secret = ''
      const saved = await pending
      if (current(request)) { const index = hosts.value.findIndex(item => item.id === saved.id); if (index < 0) hosts.value.push(saved); else hosts.value.splice(index, 1, saved) }
    }
    if (alive) emit('changed')
    if (current(request)) { cancel(); notify('儲存成功') }
  } catch (e) { if (current(request)) notify(hostErrorText(e, '無法儲存連線。請確認網址與 Token，或關閉後重新開啟再試。'), true) }
  finally { secret = ''; clearCredential(); if (current(request)) busy.value = false }
}
async function remove(): Promise<void> {
  if (busy.value || !deleting.value) return
  const row = deleting.value; clearCredential(); const request = ++generation; busy.value = true
  try {
    if (row.kind === 'screen') { if (!props.transport.instanceDelete) throw new Error('unavailable'); await props.transport.instanceDelete(row.value.id, row.value.revision) }
    else await props.transport.runtimeDeleteHost(row.value.id, row.value.revision)
    if (alive) emit('changed')
    if (current(request)) { if (row.kind === 'screen') screens.value = screens.value.filter(item => item.id !== row.value.id); else hosts.value = hosts.value.filter(item => item.id !== row.value.id); reset(); notify('本機連線設定已刪除') }
  } catch (e) { if (current(request)) notify(hostErrorText(e, '無法刪除連線設定，請關閉後重新開啟再試。'), true) }
  finally { clearCredential(); if (current(request)) busy.value = false }
}
async function test(row: Row): Promise<void> {
  if (busy.value) return
  clearCredential(); const request = ++generation; busy.value = true
  try {
    if (row.kind === 'screen') { if (!props.transport.instanceTest) throw new Error('unavailable'); await props.transport.instanceTest(row.value.id, row.value.revision) }
    else await props.transport.runtimeTestHost(row.value.id, row.value.revision)
    if (current(request)) notify(`${row.value.name}：連線測試成功`)
  } catch (e) { if (current(request)) notify(hostErrorText(e, '連線測試失敗。請確認網路、TLS 憑證及 Token。'), true) }
  finally { clearCredential(); if (current(request)) busy.value = false }
}
async function reorder(row: Row, overId: string, after: boolean): Promise<void> {
  if (busy.value || row.value.id === overId || !props.transport.runtimeReorderConnections) return
  const group = rows.value.filter(item => item.kind === row.kind)
  if (!group.some(item => item.value.id === overId)) return
  const expected = group.map(item => item.value.id), ordered = expected.filter(id => id !== row.value.id)
  ordered.splice(ordered.indexOf(overId) + (after ? 1 : 0), 0, row.value.id)
  if (ordered.every((id, index) => id === expected[index])) return
  clearCredential(); const request = ++generation; busy.value = true
  try {
    await props.transport.runtimeReorderConnections(row.kind, expected, ordered)
    if (alive) emit('changed')
    if (current(request)) {
      const byId = new Map(group.map(item => [item.value.id, item]))
      const sorted = ordered.map(id => byId.get(id)).filter((item): item is Row => !!item)
      if (row.kind === 'screen') screens.value = sorted.flatMap(item => item.kind === 'screen' ? [item.value] : [])
      else { let index = 0; hosts.value = hosts.value.map(item => { if ((item.category === 'agent' ? 'agent' : 'host') !== row.kind) return item; const next = sorted[index++]; return next && next.kind !== 'screen' ? next.value : item }) }
      notify('順序已儲存')
    }
  } catch (e) { if (current(request)) notify(hostErrorText(e, '無法儲存順序，請關閉後重新開啟再試。'), true) }
  finally { if (current(request)) busy.value = false }
}
function keyboardReorder(event: KeyboardEvent, row: Row): void {
  if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
  event.preventDefault(); event.stopPropagation()
  const group = rows.value.filter(item => item.kind === row.kind), index = group.findIndex(item => item.value.id === row.value.id)
  const next = group[index + (event.key === 'ArrowUp' ? -1 : 1)]
  if (next) void reorder(row, next.value.id, event.key === 'ArrowDown')
}
function startDrag(event: PointerEvent, row: Row): void {
  if (busy.value || event.button !== 0 || !(event.currentTarget instanceof HTMLElement)) return
  event.preventDefault(); event.stopPropagation(); event.currentTarget.setPointerCapture?.(event.pointerId)
  drag.value = { pointer: event.pointerId, row, overId: null, after: false }
}
function moveDrag(event: PointerEvent): void {
  if (!drag.value || event.pointerId !== drag.value.pointer) return
  event.preventDefault(); event.stopPropagation()
  const element = document.elementFromPoint?.(event.clientX, event.clientY)?.closest<HTMLElement>('[data-connection-id]')
  if (!element || element.dataset.connectionKind !== drag.value.row.kind) { drag.value.overId = null; return }
  drag.value.overId = element.dataset.connectionId ?? null
  drag.value.after = event.clientY > element.getBoundingClientRect().top + element.getBoundingClientRect().height / 2
  const container = dialog.value?.closest<HTMLElement>('.settings-content, .global-settings-page') ?? dialog.value
  if (container) { const rect = container.getBoundingClientRect(); if (event.clientY < rect.top + 44) container.scrollTop -= 14; else if (event.clientY > rect.bottom - 44) container.scrollTop += 14 }
}
function endDrag(event: PointerEvent, cancelled = false): void {
  if (!drag.value || event.pointerId !== drag.value.pointer) return
  event.stopPropagation(); const pending = drag.value; drag.value = null
  if (!cancelled && pending.overId) void reorder(pending.row, pending.overId, pending.after)
}
function visibility(): void { if (document.visibilityState === 'hidden') { clearPending(); dismissToast() } }
function close(): void { reset(); dismissToast(); emit('close') }
onMounted(async () => {
  document.addEventListener('visibilitychange', visibility)
  const request = generation + 1; await reload()
  if (!current(request) || formTouched.value || editing.value || name.value || baseUrl.value || credential.value || props.initialView !== 'create') return
  const prefill = getTestComputerPrefill()
  if (prefill) { name.value = prefill.name; baseUrl.value = prefill.baseUrl; credential.value = prefill.credential; testPrefillApplied.value = true }
})
onBeforeUnmount(() => { alive = false; clearPending(); dismissToast(); document.removeEventListener('visibilitychange', visibility) })
</script>

<template>
  <div class="runtime-host-backdrop" :class="{ 'embedded-host-page': embedded }">
    <section ref="dialog" tabindex="-1" class="runtime-hosts" :role="embedded ? 'region' : 'dialog'" :aria-modal="embedded ? undefined : true" aria-labelledby="runtime-host-heading">
      <header><h2 id="runtime-host-heading">{{ view === 'form' ? editing ? '編輯連線' : '新增連線' : '已儲存的連線' }}</h2><div class="header-actions"><button v-if="view === 'list' && !transferMode" type="button" class="icon-button add-connection" data-testid="runtime-create" aria-label="新增連線" title="新增連線" :disabled="busy" @click="create"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg></button><button v-if="!embedded" type="button" @click="close">關閉</button></div></header>
      <RuntimeTransfer v-if="transferMode" :mode="transferMode" :transport="transport" :hosts="hosts" @close="transferMode = null; void reload()" @changed="emit('changed'); void reload()" @picker="emit('picker', $event)" />
      <template v-else>
        <div v-if="!embedded" class="transfer-actions"><button type="button" :disabled="busy" data-testid="runtime-import" @click="reset(); transferMode = 'import'">匯入連線</button><button type="button" :disabled="busy" data-testid="runtime-export" @click="reset(); transferMode = 'export'">匯出連線</button></div>
        <div v-if="view === 'list'" data-testid="runtime-host-list">
          <p v-if="busy" class="pending" role="status">處理中…</p>
          <section v-for="group in groups" :key="group.kind" class="connection-group" :aria-label="group.title">
            <h3><ConnectionIcon :kind="group.kind" />{{ group.title }}<small>{{ group.rows.length }}</small></h3>
            <p v-if="!busy && !group.rows.length" class="empty-group">尚未儲存 {{ group.title }} 連線</p>
            <ul><li v-for="row in group.rows" :key="row.value.id" :data-connection-id="row.value.id" :data-connection-kind="row.kind" :data-host-id="row.kind !== 'screen' ? row.value.id : undefined" :class="{ dragging: drag?.row.value.id === row.value.id, 'drop-before': drag?.overId === row.value.id && !drag.after, 'drop-after': drag?.overId === row.value.id && drag.after }">
              <button type="button" class="icon-button drag-handle" data-action="reorder" :aria-label="`調整 ${row.value.name} 順序`" title="拖曳排序，或使用上下方向鍵" :disabled="busy || group.rows.length < 2 || !transport.runtimeReorderConnections" @pointerdown="startDrag($event, row)" @pointermove="moveDrag" @pointerup="endDrag($event)" @pointercancel="endDrag($event, true)" @lostpointercapture="endDrag($event, true)" @keydown="keyboardReorder($event, row)"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5h.01M16 5h.01M8 12h.01M16 12h.01M8 19h.01M16 19h.01" /></svg></button>
              <div class="connection-name"><ConnectionIcon :kind="row.kind" /><strong>{{ row.value.name }}</strong></div><span class="connection-url">{{ rowUrl(row) }}</span>
              <div class="row-actions">
                <button type="button" class="icon-button" data-action="edit" aria-label="編輯" :title="`編輯 ${row.value.name}`" :disabled="busy" @click="edit(row)"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m16 3 5 5-12 12-6 1 1-6ZM14 5l5 5" /></svg></button>
                <button type="button" class="icon-button" data-action="test" aria-label="測試" :title="`測試 ${row.value.name} 連線`" :disabled="busy" @click="test(row)"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m10 13 4-4M8 16l-1 1a4 4 0 0 1-6-6l4-4a4 4 0 0 1 6 0M16 8l1-1a4 4 0 0 1 6 6l-4 4a4 4 0 0 1-6 0" transform="translate(0 -1) scale(.95)" /></svg></button>
                <button type="button" class="icon-button delete-action" data-action="delete" aria-label="刪除" :title="`刪除 ${row.value.name}`" :disabled="busy" @click="reset(); deleting = row"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7" /></svg></button>
              </div>
            </li></ul>
          </section>
          <div v-if="deleting" class="delete-confirm" role="group" aria-label="確認刪除"><p>刪除「{{ deleting.value.name }}」的本機設定？現有遠端工作不會被終止。</p><button type="button" :disabled="busy" data-testid="confirm-delete" @click="remove">確認刪除本機設定</button><button type="button" :disabled="busy" @click="reset">取消</button></div>
        </div>
        <form v-else data-testid="runtime-host-form" :aria-describedby="error ? 'runtime-host-error' : undefined" @submit.prevent="save">
          <fieldset class="type-options" :disabled="busy"><legend>連線類型</legend><label v-for="item in kinds" :key="item.kind"><input v-model="category" type="radio" name="connection-kind" :value="item.kind" :data-testid="`connection-type-${item.kind}`" :disabled="!!editing && (editing.kind === 'screen' ? item.kind !== 'screen' : item.kind === 'screen')" @change="changeKind"><ConnectionIcon :kind="item.kind" /><span>{{ item.title }}</span></label></fieldset>
          <label>名稱<input v-model="name" data-testid="name" :disabled="busy" maxlength="128" autocomplete="off" @input="formTouched = true" /></label>
          <label>網址<input v-model="baseUrl" data-testid="base-url" :disabled="busy" maxlength="2048" type="url" :placeholder="category === 'screen' ? 'https://screen.example.test/mcp' : 'https://runtime.example.test'" autocomplete="off" spellcheck="false" @input="formTouched = true" /></label>
          <label>Token<input ref="credentialInput" v-model="credential" data-testid="credential" :disabled="busy" type="password" maxlength="1024" autocomplete="new-password" autocapitalize="off" spellcheck="false" @input="formTouched = true" /></label>
          <p v-if="editing" class="form-hint">Token 留空保留原值；變更網址時須重新輸入。</p>
          <label v-if="category !== 'screen'">顯示名稱（選填）<input v-model="displayName" data-testid="display-name" :disabled="busy" autocomplete="off" placeholder="留空使用預設名稱" @input="formTouched = true" /><small>{{ [...displayName].length }} / 20 個字元</small></label>
          <p v-if="error" id="runtime-host-error" data-testid="form-error" role="alert">{{ error }}</p><p v-if="testPrefillApplied" class="form-hint">私人測試預填：尚未儲存，請儲存後再測試。</p>
          <div class="form-actions"><button type="submit" :disabled="busy">{{ busy ? '處理中…' : '儲存' }}</button><button type="button" data-testid="runtime-form-cancel" @click="cancel">取消</button></div>
        </form>
      </template>
    </section>
    <ConnectionToast v-if="toast" :notification="toast" @dismiss="dismissToast" />
  </div>
</template>

<style scoped>
.runtime-host-backdrop { position: fixed; inset: 0; z-index: 100; background: #0008; display: flex; justify-content: center; align-items: center; padding: 16px; }
.runtime-hosts { background: var(--bg-panel, #18202b); color: var(--text, #e8edf5); width: min(900px, 100%); max-height: calc(100dvh - 32px); overflow: auto; padding: 20px; border: 1px solid var(--border, #6b7788); border-radius: 12px; box-sizing: border-box; }
header, .header-actions, .row-actions { display: flex; align-items: center; gap: 6px; } header { justify-content: space-between; gap: 12px; }
h2 { font-size: 16px; margin: 0; } h3 { display: flex; align-items: center; gap: 8px; font-size: 12px; font-weight: 550; color: var(--text-dim); margin: 20px 0 5px; } h3 small { margin-left: auto; }
button { min-height: 44px; font: inherit; background: var(--bg-raised, #26282e); color: inherit; border: 1px solid var(--border, #6b7788); border-radius: 7px; padding: 7px 12px; cursor: pointer; }
button:disabled { cursor: default; opacity: .4; } button:hover:not(:disabled) { background: var(--bg-hover, #2e3038); } :is(button, input):focus-visible { outline: 2px solid var(--accent, #5b8def); outline-offset: 2px; }
.icon-button { display: grid; place-items: center; flex: none; width: 44px; min-height: 44px; padding: 11px; border: 0; background: transparent; } .icon-button svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
.add-connection { color: var(--accent); background: var(--bg-raised); border: 1px solid var(--border); } .delete-action { color: var(--text-dim); } .delete-action:hover:not(:disabled) { color: var(--err); }
.connection-group ul { list-style: none; padding: 0; margin: 0; } .connection-group li { display: grid; grid-template-columns: 32px minmax(100px, .7fr) minmax(100px, 1.4fr) auto; align-items: center; gap: 10px; padding: 8px 0; border-bottom: 1px solid var(--border); position: relative; }
.connection-name { display: flex; align-items: center; gap: 8px; min-width: 0; font-size: 13px; } .connection-name strong { font-weight: 550; overflow-wrap: anywhere; } .connection-url { min-width: 0; overflow-wrap: anywhere; white-space: normal; font-size: 12px; color: var(--text-dim); line-height: 1.5; }
.row-actions { gap: 0; } .drag-handle { width: 32px; padding: 6px; touch-action: none; cursor: grab; color: var(--text-dim); user-select: none; } .drag-handle svg { stroke-width: 4; } .dragging { opacity: .55; }
.drop-before::before, .drop-after::after { content: ''; position: absolute; inset-inline: 0; height: 2px; background: var(--accent); pointer-events: none; } .drop-before::before { top: 0; } .drop-after::after { bottom: 0; }
.empty-group, .pending, .form-hint, small { font-size: 11px; line-height: 1.5; color: var(--text-dim); } form > label { display: flex; flex-direction: column; gap: 6px; margin: 16px 0; font-size: 12px; }
input { min-width: 0; min-height: 44px; box-sizing: border-box; font: inherit; font-size: 16px; padding: 9px; background: var(--bg, #101722); color: inherit; border: 1px solid var(--border, #6b7788); border-radius: 7px; }
.type-options { display: flex; flex-wrap: nowrap; gap: 12px; border: 0; padding: 0; margin: 18px 0; min-width: 0; } .type-options legend { margin-bottom: 8px; color: var(--text-dim); font-size: 11px; } .type-options label { display: flex; align-items: center; gap: 5px; min-height: 44px; font-size: 12px; cursor: pointer; } .type-options input { min-height: auto; width: 14px; height: 14px; margin: 0; accent-color: var(--accent); } .type-options :deep(svg) { width: 17px; height: 17px; }
.form-actions, .transfer-actions { display: flex; gap: 8px; margin-top: 16px; } .form-actions button[type=submit] { color: var(--accent); } [role=alert] { color: var(--err, #ffb7b7); }
.delete-confirm { padding: 12px; margin-top: 16px; background: var(--bg-raised); border: 1px solid var(--border); border-radius: 8px; font-size: 12px; } .delete-confirm button { margin-right: 8px; }
.embedded-host-page.runtime-host-backdrop { position: static; display: block; padding: 0; background: transparent; } .embedded-host-page .runtime-hosts { width: 100%; max-height: none; padding: 0; overflow: visible; border: 0; background: transparent; }
@media (max-width: 600px) { .runtime-host-backdrop { padding: 8px; } .runtime-hosts { padding: 16px; max-height: calc(100dvh - 16px); } .connection-group li { grid-template-columns: 28px minmax(0, 1fr) auto; gap: 4px 6px; padding: 8px 0; } .connection-url { grid-column: 2 / -1; grid-row: 2; padding-bottom: 4px; } .connection-name { font-size: 12px; } .drag-handle { grid-row: 1 / 3; width: 28px; } .type-options { gap: 9px; } .type-options label { gap: 4px; font-size: 11px; } .type-options :deep(svg) { width: 15px; height: 15px; } }
</style>
