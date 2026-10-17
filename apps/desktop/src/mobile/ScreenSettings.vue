<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { InstanceDesktop, Transport } from '../transport/types'
const props = defineProps<{ transport: Transport }>()
const emit = defineEmits<{ changed: [] }>()
const items = ref<InstanceDesktop[]>([]), editing = ref<InstanceDesktop | null>(null)
const name = ref(''), url = ref(''), token = ref(''), busy = ref(false), error = ref(''), notice = ref('')
const needsToken = computed(() => !editing.value || url.value.trim() !== editing.value.mcpUrl)
let alive = true, generation = 0
function reset() { editing.value = null; name.value = ''; url.value = ''; token.value = ''; error.value = '' }
function background() { if (document.hidden) { token.value = ''; generation++; busy.value = false } }
async function load() {
  const current = ++generation
  try { const list = await props.transport.instanceDesktops?.() ?? []; if (alive && generation === current) items.value = list }
  catch { if (alive && generation === current) error.value = '無法讀取 Screen 設定' }
}
function edit(item: InstanceDesktop) { editing.value = { ...item }; name.value = item.name; url.value = item.mcpUrl; token.value = ''; error.value = ''; notice.value = '' }
async function save() {
  if (busy.value || !props.transport.instanceSave) return
  const current = ++generation
  const input = { id: editing.value?.id, expectedRevision: editing.value?.revision, name: name.value.trim(), mcpUrl: url.value.trim(), token: token.value }
  token.value = ''; error.value = ''; notice.value = ''; busy.value = true
  try {
    await props.transport.instanceSave(input)
    if (!alive || generation !== current) return
    reset(); notice.value = 'Screen 已儲存'; busy.value = false; emit('changed'); await load()
  } catch { if (alive && generation === current) error.value = '保存失敗；請確認網址、token 與設定版本' }
  finally { input.token = ''; if (alive && generation === current) busy.value = false }
}
async function act(item: InstanceDesktop, kind: 'test' | 'delete') {
  if (busy.value) return
  const current = ++generation; busy.value = true; error.value = ''; notice.value = ''
  try {
    if (kind === 'test') await props.transport.instanceTest?.(item.id, item.revision)
    else await props.transport.instanceDelete?.(item.id, item.revision)
    if (!alive || generation !== current) return
    notice.value = kind === 'test' ? 'Screen 連線測試通過' : 'Screen 已刪除'
    if (kind === 'delete') { if (editing.value?.id === item.id) reset(); busy.value = false; emit('changed'); await load() }
  } catch { if (alive && generation === current) error.value = kind === 'test' ? 'Screen 測試失敗，請確認網路與認證' : '刪除失敗，請重新讀取設定' }
  finally { if (alive && generation === current) busy.value = false }
}
onMounted(() => { document.addEventListener('visibilitychange', background); void load() })
onBeforeUnmount(() => { alive = false; generation++; token.value = ''; document.removeEventListener('visibilitychange', background) })
</script>
<template>
  <section class="screen-settings" aria-labelledby="screen-settings-title" data-testid="screen-settings">
    <div class="settings-eyebrow">SCREENS</div><h2 id="screen-settings-title">畫面設定</h2>
    <p class="scope-note">獨立設定 instance-mcp，讓 App 直接取得畫面。連線匯出不包含 Screen 設定。</p>
    <ul v-if="items.length" class="saved-screens">
      <li v-for="item in items" :key="item.id" :data-testid="`saved-screen-${item.id}`">
        <div class="screen-detail"><strong>{{ item.name }}</strong><span>{{ item.mcpUrl }}</span></div>
        <div class="screen-actions"><button :disabled="busy" @click="act(item, 'test')">測試</button><button :disabled="busy" @click="edit(item)">編輯</button><button :disabled="busy" @click="act(item, 'delete')">刪除</button></div>
      </li>
    </ul>
    <form data-testid="screen-form" @submit.prevent="save">
      <h3>{{ editing ? '編輯 Screen' : '新增 Screen' }}</h3>
      <label>名稱<input v-model="name" data-testid="screen-name" required maxlength="128" :disabled="busy" autocomplete="off"></label>
      <label>instance-mcp URL<input v-model="url" data-testid="screen-url" type="url" required :disabled="busy" placeholder="https://screen.example.test/mcp" autocomplete="off" spellcheck="false"></label>
      <label>Token<input v-model="token" data-testid="screen-token" type="password" :required="needsToken" maxlength="1024" :disabled="busy" :placeholder="editing && !needsToken ? '留空保留原 token' : ''" autocomplete="new-password"></label>
      <p v-if="editing && needsToken" class="scope-note">網址變更後請重新填入 token。</p>
      <div class="form-actions"><button type="submit" :disabled="busy || !name.trim() || !url.trim() || (needsToken && !token)">{{ busy ? '處理中…' : '儲存 Screen' }}</button><button v-if="editing" type="button" :disabled="busy" @click="reset">取消編輯</button></div>
    </form>
    <p v-if="error" class="screen-error" role="alert">{{ error }}</p><p v-if="notice" class="scope-note" role="status">{{ notice }}</p>
  </section>
</template>
<style scoped>
.screen-settings { margin-top: 24px; padding-top: 20px; border-top: 1px solid var(--border); color: var(--text); }
.settings-eyebrow { color: var(--text-dim); font-size: 9px; letter-spacing: .14em; }
h2 { margin: 5px 0 0; font-size: 17px; font-weight: 600; } h3 { margin: 16px 0 12px; font-size: 14px; font-weight: 550; }
.scope-note { color: var(--text-dim); font-size: 11px; line-height: 1.5; }
.saved-screens { list-style: none; padding: 0; margin: 14px 0; } li { padding: 9px 0; border-bottom: 1px solid var(--border); }
.screen-detail { display: flex; flex-direction: column; gap: 4px; min-width: 0; } strong { font-size: 12px; } .screen-detail span { font-size: 10px; color: var(--text-dim); overflow-wrap: anywhere; }
.screen-actions, .form-actions { display: flex; gap: 7px; margin-top: 8px; }
button { font: inherit; font-size: 11px; cursor: pointer; min-height: 32px; padding: 4px 9px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-raised); color: var(--text); }
button:disabled { opacity: .45; cursor: default; } button:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
form label { display: flex; flex-direction: column; gap: 6px; margin: 12px 0; font-size: 11px; color: var(--text-dim); }
input { width: 100%; box-sizing: border-box; min-height: 42px; font-size: 16px; background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 7px; padding: 8px; }
.form-actions button[type=submit] { color: var(--accent); min-height: 40px; } .screen-error { color: var(--err); font-size: 11px; }
</style>
