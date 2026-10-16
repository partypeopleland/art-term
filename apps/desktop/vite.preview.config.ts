import { appInfoPlugin } from './app-info-plugin'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath } from 'node:url'

const local = (path: string) => fileURLToPath(new URL(path, import.meta.url))

// Separate root/config: the APK's normal entry and dist never include preview code.
export default defineConfig({
  root: local('./ui-preview'),
  plugins: [vue(), appInfoPlugin()],
  resolve: {
    alias: {
      '@tauri-apps/api/core': local('./ui-preview/native.ts'),
      '@tauri-apps/plugin-clipboard-manager': local('./ui-preview/clipboard.ts')
    }
  },
  define: { __ART_TERM_TEST_COMPUTER_PREFILL__: 'null' },
  server: { host: '127.0.0.1', port: 1421, strictPort: true },
  build: { outDir: '../dist-ui-preview', emptyOutDir: true }
})
