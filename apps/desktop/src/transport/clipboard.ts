// Only clipboard writes are granted to the local app; no clipboard reads.
export { writeText as writeClipboardText } from '@tauri-apps/plugin-clipboard-manager'
