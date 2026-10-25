import { invoke } from '@tauri-apps/api/core'
import type { LinkOpener } from '../terminal/links'

export function createNativeWebLinkOpener(platform: 'windows' | 'linux' | 'android'): LinkOpener {
  return url => invoke<void>(platform === 'android' ? 'plugin:mobile-history|openUrl' : 'gui_open_web_url', { url })
}
