import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import type { Plugin } from 'vite'

const reportRoot = fileURLToPath(new URL('../../docs/report/', import.meta.url))
const configPath = fileURLToPath(new URL('../../crates/art-term-gui/tauri.android.conf.json', import.meta.url))
const moduleId = 'virtual:art-term-info'
const resolvedId = `\0${moduleId}`

/** Bundle repository release notes and the Android version for offline reading. */
export function appInfoPlugin(): Plugin {
  let building = false
  return {
    name: 'art-term-app-info',
    configResolved(config) { building = config.command === 'build' },
    resolveId(id) { if (id === moduleId) return resolvedId },
    load(id) {
      if (id !== resolvedId) return
      if (building) this.addWatchFile(configPath)
      const config = JSON.parse(readFileSync(configPath, 'utf8'))
      const releases = readdirSync(reportRoot)
        .filter(name => /^\d+\.\d+\.\d+\.md$/.test(name))
        .sort((a, b) => {
          const left = a.slice(0, -3).split('.').map(Number)
          const right = b.slice(0, -3).split('.').map(Number)
          return right[0] - left[0] || right[1] - left[1] || right[2] - left[2]
        })
        .map(name => {
          const path = `${reportRoot}${name}`
          if (building) this.addWatchFile(path)
          return { version: name.slice(0, -3), content: readFileSync(path, 'utf8') }
        })
      return `export default ${JSON.stringify({ version: config.version, androidBuild: config.bundle.android.versionCode, releases })}`
    },
    configureServer(server) { server.watcher.add([reportRoot, configPath]) },
    handleHotUpdate(context) {
      if (context.file.startsWith(reportRoot) || context.file === configPath) {
        const module = context.server.moduleGraph.getModuleById(resolvedId)
        if (module) context.server.moduleGraph.invalidateModule(module)
        context.server.ws.send({ type: 'full-reload' })
        return []
      }
    }
  }
}
