import { ref } from 'vue'
import type { ITheme } from '@xterm/xterm'

export const themes = [
  { id: 'tech', name: '科技感', description: '深藍夜色 · 冷光點綴', colors: ['#0b1420', '#172638', '#62d3ff', '#b59aff'] },
  { id: 'dark', name: '深黑', description: '純粹黑灰 · 低調沉穩', colors: ['#090a0c', '#1a1c20', '#b5c4d6', '#d1bde7'] },
  { id: 'light', name: '柔光', description: '暖白霧灰 · 輕柔明亮', colors: ['#f3f2ee', '#fffefa', '#31788e', '#87669e'] },
  { id: 'retro', name: '懷舊', description: '暖褐紙色 · 琥珀與橄欖綠', colors: ['#28251f', '#363128', '#d9ad68', '#b9be83'] }
] as const
export type ThemeId = typeof themes[number]['id']
const storageKey = 'art-term.ui.theme'
function loadTheme(): ThemeId {
  try {
    const saved = localStorage.getItem(storageKey)
    if (themes.some(t => t.id === saved)) return saved as ThemeId
  } catch { /* Storage may be unavailable; the theme still works for this session. */ }
  return 'tech'
}
export const selectedTheme = ref<ThemeId>(loadTheme())
export function selectTheme(theme: ThemeId): void {
  selectedTheme.value = theme
  try { localStorage.setItem(storageKey, theme) } catch { /* Keep the in-memory preference. */ }
}

export function terminalTheme(theme: ThemeId): ITheme {
  const colors = {
    tech: { background: '#0b1420', foreground: '#dce7f4', cursor: '#62d3ff', selectionBackground: '#29435e', black: '#172638', red: '#ef939c', green: '#86cda1', yellow: '#e0bf7e', blue: '#79b8ed', magenta: '#c4a1ee', cyan: '#77d9d0', white: '#dce7f4' },
    dark: { background: '#090a0c', foreground: '#dedee2', cursor: '#dedee2', selectionBackground: '#363b46', black: '#1a1c20', red: '#e99595', green: '#a1c69a', yellow: '#d7c28f', blue: '#9fbadf', magenta: '#d1bde7', cyan: '#96c7c4', white: '#dedee2' },
    light: { background: '#f3f2ee', foreground: '#323b43', cursor: '#31788e', selectionBackground: '#d1dfe4', black: '#323b43', red: '#a74748', green: '#416c43', yellow: '#876520', blue: '#346c9b', magenta: '#876099', cyan: '#327574', white: '#59616a' },
    retro: { background: '#28251f', foreground: '#e1d3b9', cursor: '#d9ad68', selectionBackground: '#574b37', black: '#363128', red: '#dc9275', green: '#b9be83', yellow: '#d9ad68', blue: '#9cacaa', magenta: '#c6a68a', cyan: '#a6b99b', white: '#e1d3b9' }
  }[theme]
  return { ...colors, cursorAccent: colors.background, brightBlack: theme === 'light' ? '#70777c' : '#8e9397', brightRed: colors.red, brightGreen: colors.green, brightYellow: colors.yellow, brightBlue: colors.blue, brightMagenta: colors.magenta, brightCyan: colors.cyan, brightWhite: colors.foreground }
}
