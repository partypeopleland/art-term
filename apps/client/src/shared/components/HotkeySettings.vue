<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import {
  DEFAULT_PANE_HOTKEYS,
  PANE_HOTKEY_ACTIONS,
  bindingFromEvent,
  hotkeyLabel,
  paneHotkeys,
  resetPaneHotkeys,
  setPaneHotkey,
  validatePaneHotkey,
  type PaneHotkeyAction
} from '../state/hotkeys'

const recording = ref<PaneHotkeyAction | null>(null)
const error = ref('')

function startRecording(action: PaneHotkeyAction): void {
  recording.value = action
  error.value = ''
  window.addEventListener('keydown', onCaptureKey, true)
}

function stopRecording(): void {
  recording.value = null
  window.removeEventListener('keydown', onCaptureKey, true)
}

function onCaptureKey(event: KeyboardEvent): void {
  event.preventDefault()
  event.stopPropagation()
  if (event.code === 'Escape') {
    stopRecording()
    return
  }
  const binding = bindingFromEvent(event)
  const action = recording.value
  if (!action || !binding) return
  const problem = validatePaneHotkey(action, binding)
  if (problem) {
    error.value = problem
    return
  }
  setPaneHotkey(action, binding)
  error.value = ''
  stopRecording()
}

function resetOne(action: PaneHotkeyAction): void {
  setPaneHotkey(action, { ...DEFAULT_PANE_HOTKEYS[action] })
  error.value = ''
}

function resetAll(): void {
  stopRecording()
  resetPaneHotkeys()
  error.value = ''
}

onBeforeUnmount(stopRecording)
</script>

<template>
  <section class="hotkey-settings" aria-labelledby="hotkey-settings-title">
    <div class="settings-eyebrow">KEYBOARD</div>
    <h2 id="hotkey-settings-title">窗格快捷鍵</h2>
    <p class="hotkey-note">作用於目前焦點所在的連線窗格。按鍵以位置比對，Shift+= 即「+」鍵。若與輸入法切換快捷鍵衝突，可在此重新指定。</p>
    <ul class="hotkey-list">
      <li v-for="item in PANE_HOTKEY_ACTIONS" :key="item.id" class="hotkey-row" :data-testid="`hotkey-${item.id}`">
        <span class="hotkey-name">{{ item.label }}</span>
        <template v-if="recording === item.id">
          <span class="hotkey-capture">請按下新快捷鍵（Esc 取消）</span>
          <button class="btn" @click="stopRecording">取消</button>
        </template>
        <template v-else>
          <kbd class="hotkey-value">{{ hotkeyLabel(paneHotkeys[item.id]) }}</kbd>
          <button class="btn" :data-testid="`hotkey-record-${item.id}`" @click="startRecording(item.id)">變更</button>
          <button class="btn" :data-testid="`hotkey-reset-${item.id}`" @click="resetOne(item.id)">預設</button>
        </template>
      </li>
    </ul>
    <p v-if="error" class="hotkey-error" role="alert">{{ error }}</p>
    <div class="hotkey-actions">
      <button class="btn" data-testid="hotkey-reset-all" @click="resetAll">全部恢復預設</button>
    </div>
  </section>
</template>

<style scoped>
.hotkey-settings { margin-top: 28px; padding-top: 22px; border-top: 1px solid var(--border); }
.hotkey-settings h2 { margin: 5px 0 0; font-size: 18px; font-weight: 600; }
.hotkey-note { margin: 10px 0 0; font-size: 11px; color: var(--text-dim); }
.hotkey-list { list-style: none; margin: 14px 0 0; padding: 0; display: grid; gap: 8px; }
.hotkey-row { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 9px; background: var(--bg); }
.hotkey-name { flex: 1; font-size: 13px; }
.hotkey-value { font-family: inherit; font-size: 12px; padding: 3px 8px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-raised); color: var(--text); }
.hotkey-capture { flex: 1; font-size: 12px; color: var(--accent); }
.hotkey-error { margin: 10px 0 0; font-size: 12px; color: #e08080; }
.hotkey-actions { margin-top: 14px; }
.hotkey-row .btn, .hotkey-actions .btn { font-size: 12px; }
</style>
