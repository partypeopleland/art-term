<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { VoiceController, VoiceTarget } from './voice'

const props = defineProps<{
  voice: VoiceController
  target: VoiceTarget
}>()

const state = props.voice.state
const settingsOpen = ref(false)
const elapsedSeconds = ref(0)
let recordingStartedAt = 0
let timerId: ReturnType<typeof setInterval> | undefined
let resultTimeout: ReturnType<typeof setTimeout> | undefined
let heldPointer: number | null = null
let keyboardHeld = false

const elapsedLabel = computed(() => {
  const total = elapsedSeconds.value
  return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`
})

const actionLabel = computed(() => {
  switch (state.phase) {
    case 'starting': return '正在準備…'
    case 'listening': return `錄音中 ${elapsedLabel.value}`
    case 'stopping': return '辨識處理中…'
    default: return '語音輸入'
  }
})

const statusText = computed(() => state.sentText || state.notice || (state.phase === 'idle' ? '預設使用繁體中文，可在設定中允許中英文切換。' : ''))
const micDisabled = computed(() => state.phase === 'stopping' || !state.recognizerAvailable)
const micLabel = computed(() => state.phase === 'listening' ? '結束語音輸入' : '開始語音輸入')

watch(() => state.phase, (phase) => {
  if (phase === 'listening') {
    recordingStartedAt = Date.now()
    elapsedSeconds.value = 0
    clearInterval(timerId)
    timerId = setInterval(() => {
      elapsedSeconds.value = Math.floor((Date.now() - recordingStartedAt) / 1000)
    }, 250)
  } else {
    clearInterval(timerId)
    timerId = undefined
  }
}, { immediate: true })

watch(() => state.sentText, (sentText) => {
  clearTimeout(resultTimeout)
  if (sentText) resultTimeout = setTimeout(() => props.voice.clearSentText(), 5000)
})

function startOrStop(): void {
  if (state.phase === 'starting' || state.phase === 'listening') void props.voice.stop()
  else if (state.phase === 'idle') void props.voice.start(props.target)
}

function onClick(): void {
  if (state.settings.mode === 'tap') startOrStop()
}

function onPointerDown(event: PointerEvent): void {
  if (state.settings.mode !== 'hold' || micDisabled.value || state.phase !== 'idle' || heldPointer !== null || !props.target) return
  if (event.isPrimary === false) return
  if (event.pointerType === 'mouse' && event.button !== 0) return
  event.preventDefault()
  heldPointer = event.pointerId
  try { (event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId) } catch { /* pointer capture is optional in older WebViews */ }
  void props.voice.start(props.target)
}

function endPointer(event: PointerEvent): void {
  if (heldPointer !== event.pointerId) return
  heldPointer = null
  if (state.phase === 'starting' || state.phase === 'listening') void props.voice.stop()
}

function onHoldKeyDown(event: KeyboardEvent): void {
  if (state.settings.mode !== 'hold' || event.repeat || micDisabled.value || state.phase !== 'idle' || keyboardHeld) return
  keyboardHeld = true
  event.preventDefault()
  void props.voice.start(props.target)
}

function onHoldKeyUp(event: KeyboardEvent): void {
  if (state.settings.mode !== 'hold' || !keyboardHeld) return
  keyboardHeld = false
  event.preventDefault()
  if (state.phase === 'starting' || state.phase === 'listening') void props.voice.stop()
}

function toggleSettings(): void {
  settingsOpen.value = !settingsOpen.value
}

function updateMode(event: Event): void {
  const value = (event.target as HTMLInputElement).value
  props.voice.updateSettings({ mode: value === 'hold' ? 'hold' : 'tap' })
}

function updateLanguageSwitch(event: Event): void {
  props.voice.updateSettings({ allowLanguageSwitch: (event.target as HTMLInputElement).checked })
}

onBeforeUnmount(() => {
  if (heldPointer !== null || keyboardHeld) props.voice.cancel('unmount')
  clearInterval(timerId)
  clearTimeout(resultTimeout)
})
</script>

<template>
  <section class="voice-input" data-testid="voice-input" :class="`phase-${state.phase}`">
    <button
      class="voice-mic"
      :class="{ recording: state.phase === 'listening', stopping: state.phase === 'stopping' }"
      data-testid="voice-mic"
      type="button"
      :disabled="micDisabled"
      :aria-label="micLabel"
      :aria-pressed="state.phase === 'listening'"
      :aria-describedby="'voice-status'"
      @click="onClick"
      @pointerdown="onPointerDown"
      @pointerup="endPointer"
      @pointercancel="endPointer"
      @lostpointercapture="endPointer"
      @keydown.enter="onHoldKeyDown"
      @keydown.space="onHoldKeyDown"
      @keyup.enter="onHoldKeyUp"
      @keyup.space="onHoldKeyUp"
    >
      <svg class="mic-icon" viewBox="0 0 24 24" aria-hidden="true">
        <rect x="9" y="2.5" width="6" height="12" rx="3" />
        <path d="M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v3.5m-4 0h8" />
      </svg>
      <span class="mic-label">{{ actionLabel }}</span>
    </button>

    <button
      v-if="state.phase === 'starting' || state.phase === 'listening'"
      class="voice-cancel"
      type="button"
      data-testid="voice-cancel"
      @click="voice.cancel('user')"
    >取消</button>

    <div class="voice-feedback">
      <div v-if="state.partialText && state.phase === 'listening'" class="voice-partial" data-testid="voice-partial">
        {{ state.partialText }}
      </div>
      <div
        id="voice-status"
        class="voice-status"
        :class="{ error: !!state.noticeCode, sent: !!state.sentText }"
        data-testid="voice-status"
        role="status"
        aria-live="polite"
      >{{ statusText }}</div>
      <button
        v-if="state.downloadRequired"
        class="voice-download"
        type="button"
        data-testid="voice-download"
        @click="voice.downloadLanguage()"
      >{{ state.downloadProgress === null ? '下載語言包' : `下載中 ${state.downloadProgress}%` }}</button>
    </div>

    <button
      class="voice-settings-toggle"
      type="button"
      data-testid="voice-settings-toggle"
      aria-label="語音設定"
      :aria-expanded="settingsOpen"
      @click="toggleSettings"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z" />
        <path d="m19.4 13.5 1.1.8-1.2 2.1-1.3-.4a7.9 7.9 0 0 1-1.7 1l-.2 1.4h-2.4l-.4-1.3a7.4 7.4 0 0 1-2 0l-.7 1.1-2.1-1.2.4-1.3a7.9 7.9 0 0 1-1-1.7l-1.4-.2v-2.4l1.3-.4a7.4 7.4 0 0 1 0-2l-1.1-.7 1.2-2.1 1.3.4a7.9 7.9 0 0 1 1.7-1l.2-1.4h2.4l.4 1.3a7.4 7.4 0 0 1 2 0l.7-1.1 2.1 1.2-.4 1.3a7.9 7.9 0 0 1 1 1.7l1.4.2v2.4l-1.3.4a7.4 7.4 0 0 1 0 2Z" />
      </svg>
    </button>

    <div v-if="settingsOpen" class="voice-settings" data-testid="voice-settings">
      <fieldset :disabled="state.phase !== 'idle'">
        <legend>錄音方式</legend>
        <label>
          <input type="radio" name="voice-mode" value="tap" :checked="state.settings.mode === 'tap'" @change="updateMode">
          點一下開始，再點一下結束
        </label>
        <label>
          <input type="radio" name="voice-mode" value="hold" :checked="state.settings.mode === 'hold'" @change="updateMode">
          按住說話，放開結束
        </label>
      </fieldset>
      <label class="language-switch">
        <input type="checkbox" :checked="state.settings.allowLanguageSwitch" @change="updateLanguageSwitch">
        允許中英文切換
      </label>
      <p v-if="!state.languageSwitchSupported" class="voice-settings-hint">此裝置不支援辨識時切換語言。</p>
    </div>
  </section>
</template>

<style scoped>
.voice-input {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
  min-height: 72px;
  padding: 5px 8px;
  background: var(--bg-panel, #1d1f24);
  border-top: 1px solid var(--border, #33353d);
}

.voice-mic {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-width: 138px;
  min-height: 60px;
  padding: 7px 14px;
  border: 1px solid #7199e5;
  border-radius: 14px;
  background: #385b9b;
  color: #fff;
  font: inherit;
  font-size: 14px;
  font-weight: 650;
  touch-action: none;
  user-select: none;
  cursor: pointer;
}

.voice-mic:focus-visible,
.voice-settings-toggle:focus-visible,
.voice-download:focus-visible {
  outline: 2px solid #a8c5ff;
  outline-offset: 2px;
}

.voice-mic:active:not(:disabled) { background: #4369aa; }
.voice-mic.recording {
  border-color: #f0a0a0;
  background: #a34242;
  animation: mic-pulse 1.7s ease-out infinite;
}
.voice-mic.stopping { border-color: #d7ba7d; background: #745c2f; }
.voice-mic:disabled { cursor: default; opacity: .72; }

.mic-icon {
  width: 28px;
  height: 28px;
  flex: none;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.mic-label { white-space: nowrap; }

.voice-cancel {
  flex: none;
  min-width: 44px;
  min-height: 40px;
  padding: 5px 8px;
  border: 1px solid #7a4949;
  border-radius: 8px;
  background: #3c292b;
  color: #f0caca;
  font: inherit;
  font-size: 12px;
}

.voice-feedback {
  display: flex;
  flex: 1;
  flex-direction: column;
  justify-content: center;
  gap: 3px;
  min-width: 0;
  color: var(--text-dim, #9a9ca6);
  font-size: 12px;
  line-height: 1.4;
}

.voice-partial {
  max-height: 34px;
  overflow: hidden;
  color: var(--text, #d7d7dc);
  overflow-wrap: anywhere;
}

.voice-status {
  max-height: 34px;
  overflow: hidden;
  overflow-wrap: anywhere;
}
.voice-status.error { color: #e29393; }
.voice-status.sent { color: #a9d291; }

.voice-download {
  align-self: flex-start;
  min-height: 36px;
  padding: 4px 9px;
  border: 1px solid var(--border, #33353d);
  border-radius: 6px;
  background: var(--bg-raised, #26282e);
  color: var(--text, #d7d7dc);
  font: inherit;
  font-size: 12px;
}

.voice-settings-toggle {
  display: grid;
  flex: none;
  place-items: center;
  width: 44px;
  height: 44px;
  padding: 9px;
  border: 1px solid var(--border, #33353d);
  border-radius: 9px;
  background: var(--bg-raised, #26282e);
  color: var(--text, #d7d7dc);
}
.voice-settings-toggle svg {
  width: 23px;
  height: 23px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.6;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.voice-settings {
  position: absolute;
  z-index: 8;
  right: 8px;
  bottom: calc(100% + 5px);
  width: min(300px, calc(100vw - 24px));
  padding: 12px;
  border: 1px solid var(--border, #33353d);
  border-radius: 10px;
  background: var(--bg-panel, #1d1f24);
  color: var(--text, #d7d7dc);
  box-shadow: 0 8px 22px rgb(0 0 0 / 32%);
  font-size: 13px;
}

.voice-settings fieldset {
  display: grid;
  gap: 9px;
  margin: 0;
  padding: 0 0 10px;
  border: 0;
}
.voice-settings legend { margin-bottom: 8px; color: var(--text-dim, #9a9ca6); }
.voice-settings label { display: flex; align-items: center; gap: 8px; min-height: 36px; }
.voice-settings input { width: 18px; height: 18px; accent-color: #7da4f0; }
.language-switch { border-top: 1px solid var(--border, #33353d); padding-top: 8px; }
.voice-settings-hint { margin: 4px 0 0 26px; color: var(--text-dim, #9a9ca6); font-size: 11px; }

@keyframes mic-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgb(209 105 105 / 34%); }
  55% { box-shadow: 0 0 0 7px rgb(209 105 105 / 0%); }
}

@media (max-width: 400px) {
  .voice-input { gap: 7px; padding-inline: 6px; }
  .voice-mic { min-width: 126px; padding-inline: 10px; }
  .voice-feedback { font-size: 11px; }
  .voice-settings-toggle { width: 42px; height: 42px; }
}

@media (prefers-reduced-motion: reduce) {
  .voice-mic.recording { animation: none; }
}
</style>
