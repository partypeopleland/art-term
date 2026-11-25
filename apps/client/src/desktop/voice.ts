import { createVoiceController } from '../shared/voice'
import type { VoiceController, VoiceControllerOptions, VoicePlugin, VoiceTarget } from '../shared/voice'
import { createDesktopVoicePlugin, desktopVoiceRuntime } from './tauri'

export const DESKTOP_VOICE_SETTINGS_KEY = 'art-term.desktop-voice.settings.v1'

export function createDesktopVoiceController(plugin: VoicePlugin = createDesktopVoicePlugin(), options: VoiceControllerOptions = {}): VoiceController {
  return createVoiceController(plugin, {
    isTauri: desktopVoiceRuntime,
    ...options,
    settingsKey: options.settingsKey ?? DESKTOP_VOICE_SETTINGS_KEY
  })
}

export type { VoiceController, VoicePlugin, VoiceTarget }
