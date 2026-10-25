<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { VoiceController, VoiceTarget } from './voice'

const props = defineProps<{
  voice: VoiceController
  target: VoiceTarget
}>()
const emit = defineEmits<{ overlay: [open: boolean] }>()

const state = props.voice.state
const settingsOpen = ref(false)
const lightboxOpen = ref(false)
const lightboxText = ref('')
watch(lightboxOpen, open => emit('overlay', open), { flush: 'sync' })
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

const statusText = computed(() => state.sentText || state.notice)
const micDisabled = computed(() => state.phase === 'stopping' || !state.recognizerAvailable)
const micLabel = computed(() => state.phase === 'listening' ? '結束語音輸入' : '開始語音輸入')

watch(() => state.settings.previewMode, mode => { lightboxOpen.value = mode === 'lightbox' })
watch(() => [state.partialText, state.sentText], ([partial, sent]) => {
  if (partial || sent) lightboxText.value = partial || sent
})

watch(() => state.phase, (phase) => {
  if (phase === 'starting') {
    lightboxText.value = ''
    if (state.settings.previewMode === 'lightbox') lightboxOpen.value = true
  }
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

function updatePreviewMode(event: Event): void {
  props.voice.updateSettings({ previewMode: (event.target as HTMLInputElement).value === 'lightbox' ? 'lightbox' : 'inline' })
}

onBeforeUnmount(() => {
  emit('overlay', false)
  if (heldPointer !== null || keyboardHeld) props.voice.cancel('unmount')
  clearInterval(timerId)
  clearTimeout(resultTimeout)
})
</script>

<template>
  <section class="voice-input" data-testid="voice-input" :class="[`phase-${state.phase}`, { 'settings-open': settingsOpen }]">
    <button
      v-if="!settingsOpen"
      class="voice-mic"
      :class="{ recording: state.phase === 'listening', stopping: state.phase === 'stopping' }"
      data-testid="voice-mic"
      type="button"
      :disabled="micDisabled"
      :aria-label="micLabel"
      :aria-pressed="state.phase === 'listening'"
      :aria-describedby="settingsOpen || state.settings.previewMode === 'lightbox' ? undefined : 'voice-status'"
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

    <div class="voice-content">
      <div v-if="!settingsOpen && state.settings.previewMode !== 'lightbox'" class="voice-feedback" data-testid="voice-preview">
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
      <div v-else-if="settingsOpen" class="voice-settings" data-testid="voice-settings">
        <div class="voice-settings-columns">
        <fieldset :disabled="state.phase !== 'idle'">
          <legend>錄音方式</legend>
          <label>
            <input type="radio" name="voice-mode" value="tap" :checked="state.settings.mode === 'tap'" @change="updateMode">
            點按開始／結束
          </label>
          <label>
            <input type="radio" name="voice-mode" value="hold" :checked="state.settings.mode === 'hold'" @change="updateMode">
            按住說話
          </label>
        </fieldset>
        <fieldset :disabled="state.phase !== 'idle'">
          <legend>文字預覽方式</legend>
          <label><input type="radio" name="voice-preview-mode" value="inline" :checked="state.settings.previewMode !== 'lightbox'" @change="updatePreviewMode">右側預覽</label>
          <label><input type="radio" name="voice-preview-mode" value="lightbox" :checked="state.settings.previewMode === 'lightbox'" @change="updatePreviewMode">燈箱模式</label>
        </fieldset>
        </div>
        <p v-if="!state.languageSwitchSupported" class="voice-settings-hint">此裝置不支援辨識時切換語言。</p>
      </div>
      <div v-else class="voice-feedback lightbox-hint"><button class="open-lightbox" @click="lightboxOpen = true">開啟文字燈箱 ↗</button><span>{{ state.notice }}</span></div>
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
  </section>

  <div v-if="lightboxOpen && state.settings.previewMode === 'lightbox'" class="voice-lightbox-backdrop">
    <section class="voice-lightbox" role="dialog" aria-label="語音文字預覽" @keydown.esc="lightboxOpen = false">
      <header><div><span class="lightbox-eyebrow">VOICE PREVIEW</span><h3>語音文字預覽</h3></div><button class="lightbox-close" aria-label="關閉文字燈箱" @click="lightboxOpen = false">×</button></header>
      <div class="lightbox-transcript" :class="{ 'transcript-empty': !lightboxText }" aria-live="polite">{{ lightboxText }}</div>
      <footer><span>{{ state.phase === 'listening' ? '正在聆聽…' : state.phase === 'stopping' ? '辨識處理中…' : state.notice || '文字預覽' }}</span><button v-if="state.phase === 'starting' || state.phase === 'listening'" class="lightbox-stop" @click="voice.stop()">結束錄音</button></footer>
    </section>
  </div>
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

.voice-content {
  flex: 1;
  min-width: 0;
  height: 72px;
  overflow: auto;
}
.voice-content .voice-feedback { min-height: 100%; }
.voice-settings {
  color: var(--text, #d7d7dc);
  font-size: 12px;
}

.voice-settings fieldset {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0 0 4px;
  border: 0;
}
.voice-settings legend { margin-bottom: 2px; color: var(--text-dim, #9a9ca6); }
.voice-settings label { display: flex; align-items: center; gap: 6px; min-height: 24px; }
.voice-settings input { width: 18px; height: 18px; accent-color: #7da4f0; }
.language-switch { border-top: 1px solid var(--border, #33353d); padding-top: 4px; }
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

.voice-settings-columns { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.voice-settings-columns fieldset { min-width: 0; }
.voice-settings-columns fieldset + fieldset { border-left: 1px solid var(--border); padding-left: 12px; }
.voice-settings-columns legend { font-size: 11px; }
.voice-settings-columns label { gap: 4px; font-size: 11px; white-space: nowrap; }
.voice-settings-columns input { accent-color: var(--accent); width: 15px; height: 15px; }
.settings-open .voice-content { height: 72px; min-height: 0; max-height: 72px; overflow: auto; }
.settings-open .language-switch { font-size: 10px; min-height: 24px; }
.lightbox-hint { font-size: 10px; }
.open-lightbox { align-self: flex-start; padding: 0; border: 0; background: transparent; color: var(--accent); font: inherit; font-size: 11px; cursor: pointer; }
.voice-lightbox-backdrop { position: absolute; inset: 0; z-index: 110; display: flex; align-items: center; justify-content: center; padding: 24px; background: #0005; pointer-events: none; }
.voice-lightbox { pointer-events: auto; width: 100%; max-width: 520px; max-height: 65%; display: flex; flex-direction: column; padding: 18px; border: 1px solid var(--border); border-radius: 16px; background: var(--bg-panel); color: var(--text); box-shadow: 0 18px 60px #0006; }
.voice-lightbox header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 16px; }
.lightbox-eyebrow { color: var(--text-dim); font-size: 9px; letter-spacing: .12em; }
.voice-lightbox h3 { margin: 5px 0 0; font-size: 15px; font-weight: 550; }
.lightbox-close { width: 32px; height: 32px; border: 0; border-radius: 7px; background: var(--bg-raised); color: var(--text-dim); font-size: 22px; cursor: pointer; }
.lightbox-transcript { min-height: 160px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 20px; line-height: 1.8; }
.lightbox-transcript.transcript-empty { color: var(--text-dim); font-size: 16px; }
.voice-lightbox footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-top: 16px; padding-top: 12px; border-top: 1px solid var(--border); font-size: 10px; color: var(--text-dim); }
.lightbox-stop { padding: 7px 10px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-raised); color: var(--text); font: inherit; font-size: 11px; cursor: pointer; }
.voice-lightbox button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }

.voice-input { height: 84px; min-height: 84px; max-height: 84px; box-sizing: border-box; }
.voice-settings-columns { gap: 8px; }
.voice-settings-columns fieldset { padding-bottom: 0; }
.voice-settings-columns fieldset + fieldset { padding-left: 8px; }
.voice-settings-columns legend { margin-bottom: 0; line-height: 14px; }
.voice-settings-columns label { min-height: 20px; line-height: 20px; }
.settings-open .language-switch { min-height: 18px; padding-top: 1px; line-height: 18px; }
.settings-open .language-switch input { width: 14px; height: 14px; }

.voice-settings-toggle { border: 0; background: transparent; }
</style>
