<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import type { RuntimeHostDto, Transport, TransferPreview, TransferEntry, TransferAction, InstanceDesktop } from '../transport/types'
import ConnectionToast from './ConnectionToast.vue'
import { asConnectionError } from '../transport/types'
const props = defineProps<{ transport: Transport; hosts: RuntimeHostDto[]; mode: 'import' | 'export' }>()
const emit = defineEmits<{ close: []; changed: []; picker: [active: boolean] }>()
const root = ref<HTMLElement | null>(null)
const selected = ref<string[]>([])
const screens = ref<InstanceDesktop[]>([])
const inventoryLoading = ref(props.mode === 'export')
const inventoryError = ref(false)
const includeCredentials = ref(true)
const password = ref('')
const confirm = ref('')
const supplements = reactive<Record<string, string>>({})
const actions = reactive<Record<string, TransferAction>>({})
const preview = ref<TransferPreview | null>(null)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const picking = ref(false)
let generation = 0
let alive = true
let expiry: ReturnType<typeof setTimeout> | undefined
const byteLength = (s: string) => new TextEncoder().encode(s).length
const passwordValid = computed(() => byteLength(password.value) >= 1 && byteLength(password.value) <= 1024)
const canExport = computed(() => !inventoryLoading.value && !inventoryError.value && selected.value.length > 0 && (!includeCredentials.value || (passwordValid.value && password.value === confirm.value)))
const actionable = computed(() => preview.value?.entries.some(e => actions[e.key] !== 'skip') ?? false)
function hasSecret(entry: TransferEntry): boolean { return entry.kind === 'screen' ? entry.hasToken : entry.hasCredential }
function validSupplement(entry: TransferEntry): boolean {
  const value = supplements[entry.key] ?? ''
  return byteLength(value) > 0 && byteLength(value) <= 1024 && !/[\s\x00-\x1f\x7f]/u.test(value) && (entry.kind !== 'screen' || /^[!-~]+$/.test(value))
}
const canApply = computed(() => actionable.value && preview.value?.entries.every(e => actions[e.key] !== 'add' || hasSecret(e) || validSupplement(e)))
async function loadInventory(): Promise<void> {
  inventoryLoading.value = true; inventoryError.value = false
  try {
    const value = await props.transport.instanceDesktops?.() ?? []
    if (alive) screens.value = value
  } catch { if (alive) inventoryError.value = true }
  finally { if (alive) inventoryLoading.value = false }
}
function clearSecrets(): void {
  password.value = ''; confirm.value = ''
  for (const key of Object.keys(supplements)) delete supplements[key]
  root.value?.querySelectorAll<HTMLInputElement>('input[type=password]').forEach(input => { input.value = '' })
}
function dropPreview(): void {
  preview.value = null
  for (const key of Object.keys(actions)) delete actions[key]
  if (expiry) clearTimeout(expiry)
}
function fail(e: unknown): void {
  const kind = asConnectionError(e)?.kind
  error.value = kind === 'auth' ? '密碼錯誤或檔案已損壞。請重新選檔。' : kind === 'capacity' ? '檔案或連線數量超過上限。' : kind === 'busy' ? '另一個檔案操作正在進行，請稍候。' : kind === 'storage' ? '檔案讀寫失敗。輸出檔可能不完整，請刪除後重新匯出。' : '操作未完成，可能已逾時或設定已變更。請重新選檔。'
}
function setPicker(value: boolean): void { picking.value = value; emit('picker', value) }
function install(value: TransferPreview): void {
  dropPreview()
  if (value.status === 'cancelled') { notice.value = '已取消選檔。'; return }
  preview.value = value
  for (const e of value.entries) actions[e.key] = e.matchStatus === 'new' && hasSecret(e) ? 'add' : 'skip'
  expiry = setTimeout(() => { generation++; clearSecrets(); dropPreview(); busy.value = false; error.value = '預覽已逾時。請重新選檔。'; void props.transport.transferCancel() }, 120_000)
  void nextTick(() => root.value?.querySelector<HTMLInputElement | HTMLButtonElement>('input, button')?.focus())
}
async function pick(): Promise<void> {
  if (busy.value) return
  generation++; const request = generation
  clearSecrets(); dropPreview(); error.value = ''; notice.value = ''; busy.value = true; setPicker(true)
  try {
    const value = await props.transport.transferPickImport()
    if (alive && request === generation && !document.hidden) install(value)
    else void props.transport.transferCancel()
  } catch (e) { if (alive && request === generation) fail(e) }
  finally { setPicker(false); if (alive && request === generation) busy.value = false }
}
async function unlock(): Promise<void> {
  if (busy.value || !passwordValid.value || !preview.value?.token) return
  const token = preview.value.token; let secret = password.value
  clearSecrets(); dropPreview(); const request = ++generation; busy.value = true; error.value = ''
  try {
    const pending = props.transport.transferUnlock(token, secret); secret = ''
    const value = await pending
    if (alive && request === generation && !document.hidden) install(value)
    else void props.transport.transferCancel()
  } catch (e) { if (alive && request === generation) fail(e) }
  finally { secret = ''; clearSecrets(); if (alive && request === generation) busy.value = false }
}
async function exportFile(): Promise<void> {
  if (busy.value || !canExport.value) return
  let secret = includeCredentials.value ? password.value : ''
  clearSecrets(); const request = ++generation; busy.value = true; error.value = ''; notice.value = ''; setPicker(true)
  try {
    const pending = props.transport.transferExport({ selected: [...props.hosts.filter(h => selected.value.includes('host:' + h.id)).map(h => ({ id: h.id, expectedRevision: h.revision })), ...screens.value.filter(s => selected.value.includes('screen:' + s.id)).map(s => ({ kind: 'screen' as const, id: s.id, expectedRevision: s.revision }))], includeCredentials: includeCredentials.value, password: secret }); secret = ''
    const value = await pending
    if (alive && request === generation) notice.value = value.status === 'saved' ? '連線檔案已儲存。請自行搬移到另一台裝置。' : '已取消儲存。'
  } catch (e) { if (alive && request === generation) fail(e) }
  finally { secret = ''; clearSecrets(); setPicker(false); if (alive && request === generation) busy.value = false }
}
async function apply(): Promise<void> {
  if (busy.value || !canApply.value || !preview.value?.token) return
  const token = preview.value.token
  const choices = preview.value.entries.map(e => ({ key: e.key, action: actions[e.key], ...(actions[e.key] === 'add' && !hasSecret(e) ? { credential: supplements[e.key] ?? '' } : {}) }))
  clearSecrets(); dropPreview(); const request = ++generation; busy.value = true; error.value = ''
  try {
    const pending = props.transport.transferApply(token, choices)
    choices.forEach(c => { if ('credential' in c) c.credential = '' })
    const result = await pending
    if (alive) emit('changed')
    if (alive && request === generation) notice.value = `已新增 ${result.added} 筆、更新 ${result.updated} 筆、略過 ${result.skipped} 筆連線。`
  } catch (e) { if (alive && request === generation) fail(e) }
  finally { clearSecrets(); if (alive && request === generation) busy.value = false }
}
function selectEntry(e: TransferEntry, checked: boolean): void {
  actions[e.key] = checked ? (e.matchStatus === 'new' ? 'add' : 'update') : 'skip'
  if (!checked) delete supplements[e.key]
}
function actionText(e: TransferEntry): string {
  if (e.matchStatus === 'ambiguous') return '這台裝置有多筆相同網址，請先整理；本次只能略過。'
  const secretName = e.kind === 'screen' ? 'Screen token' : '管理憑證'
  if (actions[e.key] === 'update') return hasSecret(e) ? `將替換這台裝置的${secretName}與設定。` : e.kind === 'screen' ? '保留這台裝置現有 Screen token，更新名稱與網址。' : '保留這台裝置現有憑證，更新名稱與能力。'
  if (e.matchStatus === 'duplicate') return '這台裝置已有相同網址，預設略過。'
  return hasSecret(e) ? `含${secretName}，可在這台裝置還原。` : `需要${secretName}才能新增；也可略過。`
}
function close(): void { generation++; clearSecrets(); dropPreview(); void props.transport.transferCancel(); emit('close') }
function background(): void {
  clearSecrets()
  if (!picking.value) { generation++; dropPreview(); busy.value = false; error.value = '背景切換已清除預覽，請重新選檔。'; void props.transport.transferCancel() }
}
function visibility(): void { if (document.hidden) background() }
onMounted(() => { if (props.mode === 'export') void loadInventory(); document.addEventListener('visibilitychange', visibility); window.addEventListener('blur', background) })
onBeforeUnmount(() => { alive = false; generation++; clearSecrets(); dropPreview(); setPicker(false); void props.transport.transferCancel(); document.removeEventListener('visibilitychange', visibility); window.removeEventListener('blur', background) })
</script>

<template>
  <div ref="root" class="runtime-transfer" data-testid="runtime-transfer">
    <div class="transfer-heading"><h3>{{ mode === 'export' ? '匯出連線' : '匯入連線' }}</h3><button type="button" @click="close">返回管理</button></div>
    <ConnectionToast v-if="error || notice" :notification="{ message: error || notice, error: !!error }" @dismiss="error = ''; notice = ''" />
    <p v-if="busy" role="status">{{ picking ? '請在系統視窗選擇檔案；加密處理可能需要一些時間。' : '正在處理連線檔案，請稍候…' }}</p>
    <form v-if="mode === 'export'" @submit.prevent="exportFile">
      <fieldset :disabled="busy || inventoryLoading || inventoryError"><legend>選擇要搬移的連線</legend>
        <p v-if="inventoryLoading" role="status">讀取可匯出的連線與 Screens…</p>
        <p v-else-if="inventoryError" role="alert">無法讀取 Screens，請重試後再匯出。</p>
        <p v-else-if="!hosts.length && !screens.length">目前沒有連線或 Screen 可匯出。請先新增。</p>
        <h4 v-if="hosts.length">Computers / Agents</h4>
        <label v-for="host in hosts" :key="host.id" class="choice"><input v-model="selected" type="checkbox" :value="'host:' + host.id" /><span><strong>{{ host.name }}</strong><small>{{ host.category === 'agent' ? 'Agent' : 'Host' }} · {{ host.baseUrl }}</small></span></label>
        <h4 v-if="screens.length">Screens</h4>
        <label v-for="screen in screens" :key="'screen:' + screen.id" class="choice"><input v-model="selected" type="checkbox" :value="'screen:' + screen.id" /><span><strong>{{ screen.name }}</strong><small>Screen · {{ screen.mcpUrl }}</small></span></label>
      </fieldset>
      <button v-if="inventoryError" type="button" :disabled="busy || inventoryLoading" @click="loadInventory">重新讀取 Screens</button>
      <label class="choice"><input v-model="includeCredentials" type="checkbox" :disabled="busy" @change="clearSecrets" /><span>包含管理憑證與 Screen token，以密碼加密整份檔案</span></label>
      <template v-if="includeCredentials">
        <p>另一台裝置輸入相同密碼即可還原。建議使用長且不重複的密碼，並另外保管。</p>
        <label>檔案密碼<input v-model="password" data-testid="transfer-password" type="password" autocomplete="new-password" autocapitalize="off" :spellcheck="false" :disabled="busy" maxlength="1024" /></label>
        <label>再次輸入密碼<input v-model="confirm" data-testid="transfer-confirm" type="password" autocomplete="new-password" autocapitalize="off" :spellcheck="false" :disabled="busy" maxlength="1024" /></label>
        <p v-if="confirm && password !== confirm" role="status">兩次密碼不一致。</p>
      </template>
      <p v-else>一般 JSON 會明文顯示名稱、網址與能力；不含管理憑證或 Screen token。新裝置匯入時須逐筆補入。</p>
      <button type="submit" :disabled="busy || !canExport">選擇位置並儲存</button>
    </form>
    <template v-else>
      <p v-if="!preview">選擇從另一台裝置匯出的連線檔案，確認項目後才會寫入這台裝置。</p>
      <button type="button" data-testid="transfer-pick" :disabled="busy" @click="pick">{{ preview ? '重新選檔' : '選擇連線檔案' }}</button>
      <form v-if="preview?.status === 'requiresPassword'" @submit.prevent="unlock">
        <label>檔案密碼<input v-model="password" data-testid="transfer-password" type="password" autocomplete="new-password" autocapitalize="off" :spellcheck="false" :disabled="busy" maxlength="1024" /></label>
        <button type="submit" :disabled="busy || !passwordValid">解鎖並預覽</button>
      </form>
      <form v-if="preview?.status === 'preview'" @submit.prevent="apply">
        <p>預覽保留兩分鐘。相同網址預設略過；請明確選擇要更新的項目。</p>
        <ul class="preview-list"><li v-for="entry in preview.entries" :key="entry.key">
          <strong>{{ entry.name }}</strong><small>{{ entry.kind === 'screen' ? 'Screen' : entry.metadata.category === 'agent' ? 'Agent' : 'Host' }} · {{ entry.kind === 'screen' ? entry.mcpUrl : entry.origin }}</small>
          <label class="choice"><input type="checkbox" :data-testid="'transfer-select-' + entry.key" :checked="actions[entry.key] !== 'skip'" :disabled="busy || entry.matchStatus === 'ambiguous'" @change="selectEntry(entry, ($event.target as HTMLInputElement).checked)" /><span>{{ entry.matchStatus === 'duplicate' ? '選擇更新這台裝置的連線' : '選擇匯入這筆連線' }}</span></label>
          <p>{{ actionText(entry) }}</p>
          <label v-if="entry.matchStatus === 'new' && !hasSecret(entry) && actions[entry.key] === 'add'">{{ entry.kind === 'screen' ? 'Screen token' : '管理憑證' }}<input v-model="supplements[entry.key]" :data-testid="'transfer-credential-' + entry.key" type="password" :disabled="busy" autocomplete="new-password" autocapitalize="off" :spellcheck="false" maxlength="1024" /></label>
        </li></ul>
        <button type="submit" data-testid="transfer-apply" :disabled="busy || !canApply">匯入選定連線</button>
      </form>
    </template>
  </div>
</template>

<style scoped>
.runtime-transfer { color: var(--text, #e8edf5); }
.transfer-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-top: 20px; }
h3 { margin: 0; font-size: 17px; } p { line-height: 1.6; } small { display: block; line-height: 1.5; color: var(--text-muted, #a8abb7); overflow-wrap: anywhere; margin-top: 4px; }
label { display: flex; flex-direction: column; gap: 6px; margin: 12px 0; }
.choice { flex-direction: row; align-items: center; gap: 12px; min-height: 44px; }
.choice input { flex: 0 0 20px; width: 20px; height: 20px; min-height: 20px; accent-color: var(--accent, #5b8def); }
fieldset { border: 0; padding: 0; margin: 16px 0 24px; } legend { font-weight: 600; }
input, select { min-height: 44px; box-sizing: border-box; width: 100%; font: inherit; padding: 9px; color: inherit; background: var(--bg, #1a1b20); border: 1px solid var(--border, #6b7788); border-radius: 4px; caret-color: var(--accent, #5b8def); }
button { min-height: 44px; font: inherit; padding: 8px 12px; border: 1px solid var(--border, #6b7788); border-radius: 4px; background: var(--bg-raised, #26282e); color: inherit; cursor: pointer; }
button[type=submit] { background: var(--accent, #5b8def); color: #fff; border-color: transparent; margin: 12px 0; }
button:hover:not(:disabled) { filter: brightness(1.12); } button:active:not(:disabled) { filter: brightness(.92); }
button:disabled { opacity: .55; cursor: default; }
:is(button,input,select):focus-visible { outline: 2px solid var(--accent, #5b8def); outline-offset: 3px; }
.preview-list { list-style: none; margin: 16px 0; padding: 0; }
.preview-list li { padding: 16px 0; border-bottom: 1px solid var(--border, #465365); overflow-wrap: anywhere; }
[role=alert] { color: #ffb7b7; }
@media(max-width:480px) { .transfer-heading { align-items: flex-start; } .transfer-heading button { flex-shrink: 0; } }
</style>
