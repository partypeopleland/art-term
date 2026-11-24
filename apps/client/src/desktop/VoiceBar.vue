<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { VoiceController, VoiceTarget } from '../shared/voice'

const props = defineProps<{
  voice: VoiceController
  target: VoiceTarget | null
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

const statusText = computed(() => {
  if (state.sentText || state.notice) return state.sentText || state.notice
  if (!state.pluginPresent) return '語音輸入僅在 Windows App 內可用。'
  if (!props.target) return '請先選擇已連線的分頁。'
  return ''
})
const micDisabled = computed(() => state.phase === 'stopping' || !state.recognizerAvailable || !props.target)
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
  if (!props.target) return
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
  try { (event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId) } catch { /* pointer capture is optional */ }
  void props.voice.start(props.target)
}

function endPointer(event: PointerEvent): void {
  if (heldPointer !== event.pointerId) return
  heldPointer = null
  if (state.phase === 'starting' || state.phase === 'listening') void props.voice.stop()
}

function onHoldKeyDown(event: KeyboardEvent): void {
  if (state.settings.mode !== 'hold' || event.repeat || micDisabled.value || state.phase !== 'idle' || keyboardHeld || !props.target) return
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

onBeforeUnmount(() => {
  if (heldPointer !== null || keyboardHeld) props.voice.cancel('unmount')
  clearInterval(timerId)
  clearTimeout(resultTimeout)
})
</script>

<template>
  <section class="desktop-voice" data-testid="desktop-voice" :class="[`phase-${state.phase}`, { 'settings-open': settingsOpen }]">
    <button
      v-if="!settingsOpen"
      class="desktop-voice-mic"
      :class="{ recording: state.phase === 'listening', stopping: state.phase === 'stopping' }"
      data-testid="desktop-voice-mic"
      type="button"
      :disabled="micDisabled"
      :aria-label="micLabel"
      :aria-pressed="state.phase === 'listening'"
      aria-describedby="desktop-voice-status"
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
      class="desktop-voice-cancel"
      type="button"
      data-testid="desktop-voice-cancel"
      @click="voice.cancel('user')"
    >取消</button>

    <div class="desktop-voice-content">
      <template v-if="!settingsOpen">
        <div v-if="state.partialText && state.phase === 'listening'" class="desktop-voice-partial" data-testid="desktop-voice-partial">
          {{ state.partialText }}
        </div>
        <div
          id="desktop-voice-status"
          class="desktop-voice-status"
          :class="{ error: !!state.noticeCode, sent: !!state.sentText }"
          data-testid="desktop-voice-status"
          role="status"
          aria-live="polite"
        >{{ statusText }}</div>
      </template>
      <div v-else class="desktop-voice-settings" data-testid="desktop-voice-settings">
        <fieldset :disabled="state.phase !== 'idle'">
          <legend>錄音方式</legend>
          <label>
            <input type="radio" name="desktop-voice-mode" value="tap" :checked="state.settings.mode === 'tap'" @change="updateMode">
            點按開始／結束
          </label>
          <label>
            <input type="radio" name="desktop-voice-mode" value="hold" :checked="state.settings.mode === 'hold'" @change="updateMode">
            按住說話
          </label>
        </fieldset>
        <p class="desktop-voice-hint">預設使用繁體中文（zh-TW）Windows 裝置端辨識。</p>
      </div>
    </div>

    <button
      class="desktop-voice-settings-toggle"
      type="button"
      data-testid="desktop-voice-settings-toggle"
      aria-label="語音設定"
      :aria-expanded="settingsOpen"
      @click="toggleSettings"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z" />
        <path d="m19.4 13.5 1.1.8-1.2 2.1-1.3-.4a7.9 7.9 0 0 1-1.7 1l-.2 1.4h-2.4l-.4-1.3a7.4 7.4 0 0 1-2 0l-.7 1.1-2.1-1.2.4-1.3a7.9 7.9 0 0 1-1-1.7l-1.4-.2v-2.4l1.3-.4a7.4 7.4 0 0 1 0-2l-1.1-.7 1.2-2.1 1.3.4a7.9 7.9 0 0 1 1.7-1l.2-1.4h2.4l.4 1.3a7.4 7.4 0 0 1 2 0l.7-1.1 2.1 1.2-.4 1.3a7.9 7.9 0 0 1 1 1.7l1.4.2v2.4l-1.3.4a7.4 7.4 0 0 1 0 2Z" />
      </svg>
    </button>
  </section>
</template>

<style scoped>
.desktop-voice {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
  min-height: 52px;
  padding: 5px 12px;
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
}

.desktop-voice-mic {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-width: 132px;
  min-height: 40px;
  padding: 5px 12px;
  border: 1px solid var(--accent);
  border-radius: 9px;
  background: var(--bg-raised);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  font-weight: 600;
  touch-action: none;
  user-select: none;
  cursor: pointer;
}

.desktop-voice-mic:focus-visible,
.desktop-voice-settings-toggle:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.desktop-voice-mic:active:not(:disabled) { background: var(--bg-hover); }
.desktop-voice-mic.recording {
  border-color: #f0a0a0;
  background: #a34242;
  color: #fff;
  animation: desktop-mic-pulse 1.7s ease-out infinite;
}
.desktop-voice-mic.stopping { border-color: #d7ba7d; background: #745c2f; color: #fff; }
.desktop-voice-mic:disabled { cursor: default; opacity: .65; }

.mic-icon {
  width: 20px;
  height: 20px;
  flex: none;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.8;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.mic-label { white-space: nowrap; }

.desktop-voice-cancel {
  flex: none;
  min-width: 44px;
  min-height: 36px;
  padding: 4px 8px;
  border: 1px solid #7a4949;
  border-radius: 7px;
  background: #3c292b;
  color: #f0caca;
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.desktop-voice-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
  color: var(--text-dim);
  font-size: 12px;
  line-height: 1.4;
}

.desktop-voice-partial {
  max-height: 30px;
  overflow: hidden;
  color: var(--text);
  overflow-wrap: anywhere;
}

.desktop-voice-status {
  max-height: 30px;
  overflow: hidden;
  overflow-wrap: anywhere;
}
.desktop-voice-status.error { color: #e29393; }
.desktop-voice-status.sent { color: #a9d291; }

.desktop-voice-settings {
  color: var(--text);
  font-size: 12px;
}
.desktop-voice-settings fieldset {
  display: flex;
  gap: 16px;
  margin: 0;
  padding: 0;
  border: 0;
}
.desktop-voice-settings legend { margin-bottom: 2px; color: var(--text-dim); }
.desktop-voice-settings label { display: flex; align-items: center; gap: 6px; min-height: 22px; }
.desktop-voice-settings input { width: 16px; height: 16px; accent-color: var(--accent); }
.desktop-voice-hint { margin: 2px 0 0; color: var(--text-dim); font-size: 11px; }

.desktop-voice-settings-toggle {
  display: grid;
  flex: none;
  place-items: center;
  width: 40px;
  height: 40px;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-raised);
  color: var(--text-dim);
  cursor: pointer;
}
.desktop-voice-settings-toggle svg {
  width: 20px;
  height: 20px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.6;
  stroke-linecap: round;
  stroke-linejoin: round;
}

@keyframes desktop-mic-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgb(209 105 105 / 34%); }
  55% { box-shadow: 0 0 0 6px rgb(209 105 105 / 0%); }
}

@media (prefers-reduced-motion: reduce) {
  .desktop-voice-mic.recording { animation: none; }
}
</style>
