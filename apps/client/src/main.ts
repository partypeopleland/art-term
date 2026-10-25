// Composition root — the ONLY place the Tauri adapter is wired in.
// The shared App mounts identically with any Transport implementation.
import { createApp } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import App from './desktop/App.vue'
import { createTauriTransport } from './desktop/tauri'
import { createNativeWebLinkOpener } from './shared/transport/browser'
import { createDesktopXtermTerminal, createMobileXtermTerminal } from './shared/terminal/handle'
import './shared/style.css'

async function mountApplication() {
  const platform = await invoke<string>('gui_platform')
  if (platform === 'android') {
    const [{ default: MobileApp }, { createMobileTauriTransport }] = await Promise.all([
      import('./mobile/MobileApp.vue'),
      import('./mobile/tauri')
    ])
    createApp(MobileApp, {
      transport: createMobileTauriTransport(),
      terminalFactory: (el: HTMLElement) => createMobileXtermTerminal(el, createNativeWebLinkOpener('android'))
    }).mount('#app')
  } else if (platform === 'windows') {
    createApp(App, {
      transport: createTauriTransport(),
      terminalFactory: (el: HTMLElement) => createDesktopXtermTerminal(el, createNativeWebLinkOpener('windows'))
    }).mount('#app')
  } else {
    throw new Error('Unsupported platform')
  }
}

void mountApplication().catch(() => {
  const root = document.getElementById('app')
  if (root) {
    root.textContent = 'art-term 無法啟動，請重新開啟 App。'
  }
})
