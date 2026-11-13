// Desktop grants read-text for explicit terminal paste; Android keeps write-only.
export { readText as readClipboardText, writeText as writeClipboardText } from '@tauri-apps/plugin-clipboard-manager'
