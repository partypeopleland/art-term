import { appInfoPlugin } from './app-info-plugin'
import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import { lstatSync, readFileSync } from 'node:fs'
import { isAbsolute } from 'node:path'
import { parseTestComputerPrefill } from './src/test-computer-prefill'

function privateTestPrefill() {
  const enabled = process.env.ART_TERM_ENABLE_TEST_COMPUTER_PREFILL
  const file = process.env.ART_TERM_TEST_COMPUTER_PREFILL_FILE
  if (enabled === undefined && file === undefined) return null
  // Never include the path or private input in build errors.
  try {
    if (enabled !== '1' || !file || !isAbsolute(file)) throw new Error()
    const stat = lstatSync(file)
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 4096) throw new Error()
    const value = parseTestComputerPrefill(JSON.parse(readFileSync(file, 'utf8')))
    if (!value) throw new Error()
    return value
  } catch { throw new Error('Invalid private test Computer prefill input') }
}

const testPrefill = privateTestPrefill()

export default defineConfig({
  plugins: [vue(), appInfoPlugin()],
  base: './',
  define: { __ART_TERM_TEST_COMPUTER_PREFILL__: JSON.stringify(testPrefill) },
  server: {
    strictPort: true,
    port: 1420
  },
  build: {
    target: 'es2022',
    outDir: 'dist',
    sourcemap: testPrefill === null
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.test.ts']
  }
})
