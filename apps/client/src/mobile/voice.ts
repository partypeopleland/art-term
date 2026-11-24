import { addPluginListener, invoke, isTauri } from '@tauri-apps/api/core'
import {
  createVoiceController as createSharedVoiceController,
  loadVoiceSettings as loadSharedVoiceSettings,
  DEFAULT_VOICE_SETTINGS,
  sanitizeVoiceText
} from '../shared/voice'
import type {
  VoiceController,
  VoiceControllerOptions,
  VoiceControllerState,
  VoiceEvent,
  VoicePermission,
  VoicePhase,
  VoicePlugin,
  VoiceSettings,
  VoiceStatus,
  VoiceTarget
} from '../shared/voice'

export type {
  VoiceController,
  VoiceControllerOptions,
  VoiceControllerState,
  VoiceEvent,
  VoiceMode,
  VoicePermission,
  VoicePhase,
  VoicePlugin,
  VoiceSettings,
  VoiceStatus,
  VoiceTarget
} from '../shared/voice'
export { DEFAULT_VOICE_SETTINGS, sanitizeVoiceText }

export const VOICE_SETTINGS_KEY = 'art-term.mobile-voice.settings.v1'

export function loadVoiceSettings(storage?: Pick<Storage, 'getItem'>) {
  return loadSharedVoiceSettings(storage, VOICE_SETTINGS_KEY)
}

export function createVoiceController(plugin: VoicePlugin, options: VoiceControllerOptions = {}): VoiceController {
  return createSharedVoiceController(plugin, {
    isTauri,
    ...options,
    settingsKey: options.settingsKey ?? VOICE_SETTINGS_KEY
  })
}

export function createTauriVoicePlugin(): VoicePlugin {
  return {
    status: () => invoke<VoiceStatus>('plugin:mobile-voice|status'),
    requestPermission: () => invoke<{ permission: VoicePermission }>('plugin:mobile-voice|requestPermission'),
    downloadLanguage: language => invoke<{ started: boolean }>('plugin:mobile-voice|downloadLanguage', { language }),
    start: args => invoke('plugin:mobile-voice|start', args),
    stop: args => invoke('plugin:mobile-voice|stop', args),
    cancel: args => invoke('plugin:mobile-voice|cancel', args),
    listen: async callback => {
      const listener = await addPluginListener<VoiceEvent>('mobile-voice', 'voice', callback)
      return () => { void listener.unregister() }
    }
  }
}
