<script setup lang="ts">
import { computed, ref } from 'vue'
import appInfo from 'virtual:art-term-info'

const props = withDefaults(defineProps<{ platform?: 'android' | 'windows' }>(), { platform: 'android' })
const currentVersion = computed(() => props.platform === 'windows' ? appInfo.windowsVersion : appInfo.version)
const selectedVersion = ref<string | null>(null)
const selectedRelease = computed(() => appInfo.releases.find(release => release.version === selectedVersion.value))
const highlights = (content: string) => content.split('\n').filter(line => /^- /.test(line)).slice(0, 3).map(line => line.replace(/^- /, '').replace(/\*\*/g, ''))
function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')
}
function inline(text: string): string {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label: string, href: string) => {
      try {
        const url = new URL(href.replace(/&amp;/g, '&'), `https://github.com/partypeopleland/art-term/blob/main/docs/report/${selectedVersion.value}.md`)
        return /^https?:$/.test(url.protocol) ? `<a href="${escapeHtml(url.href)}" target="_blank" rel="noreferrer">${label}</a>` : label
      } catch { return label }
    })
}
const documentBlocks = computed(() => {
  const blocks: { kind: 'h1' | 'h2' | 'h3' | 'p' | 'list'; text: string; items: string[] }[] = []
  for (const line of (selectedRelease.value?.content ?? '').split('\n')) {
    if (!line.trim()) continue
    const heading = line.match(/^(#{1,3}) (.*)$/)
    if (heading) blocks.push({ kind: `h${heading[1].length}` as 'h1' | 'h2' | 'h3', text: heading[2], items: [] })
    else if (line.startsWith('- ')) {
      if (blocks.at(-1)?.kind !== 'list') blocks.push({ kind: 'list', text: '', items: [] })
      blocks.at(-1)!.items.push(line.slice(2))
    } else blocks.push({ kind: 'p', text: line, items: [] })
  }
  return blocks
})
</script>

<template>
  <main class="app-homepage">
    <template v-if="!selectedRelease">
      <section class="app-intro">
        <div class="app-home-mark" aria-hidden="true">&gt;_</div>
        <div><span class="eyebrow">YOUR REMOTE WORKSPACE</span><h1>art-term<span>.</span></h1><p>遠端 Shell、工作助理與共享畫面，隨手連接。</p></div>
      </section>
      <section class="version-card"><div><span class="eyebrow">CURRENT VERSION</span><h2>當前版本</h2></div><div class="version-value">{{ currentVersion }}<small>{{ platform === 'windows' ? 'Windows' : `Android · Build ${appInfo.androidBuild}` }}</small></div></section>
      <p class="art-note"><span class="art-note-cursor" aria-hidden="true">▍</span><span class="art-note-author">art說:</span><span class="art-note-message">有bug是很正常的~</span><span class="art-note-spark" aria-hidden="true">✦</span></p>
      <header class="release-heading"><h2>近期更新</h2><span>RELEASE NOTES</span></header>
      <button v-for="release in appInfo.releases" :key="release.version" class="release-card" @click="selectedVersion = release.version">
        <div class="release-title"><strong>v{{ release.version }}</strong><span v-if="release.version === currentVersion" class="latest-badge">目前版本</span><span class="release-arrow" aria-hidden="true">↗</span></div>
        <ul v-if="highlights(release.content).length"><li v-for="highlight in highlights(release.content)" :key="highlight">{{ highlight }}</li></ul>
        <span class="read-release">閱讀完整更新說明 →</span>
      </button>
    </template>
    <template v-else>
      <button class="release-back" @click="selectedVersion = null">← 返回軟體資訊</button>
      <article class="release-document">
        <template v-for="(block, index) in documentBlocks" :key="index">
          <ul v-if="block.kind === 'list'"><li v-for="(item, itemIndex) in block.items" :key="itemIndex" v-html="inline(item)" /></ul>
          <component :is="block.kind" v-else v-html="inline(block.text)" />
        </template>
      </article>
    </template>
  </main>
</template>

<style scoped>
.app-homepage { max-width: 640px; margin: 0 auto; padding: 24px 16px 32px; color: var(--text); }
.app-intro { display: flex; gap: 14px; align-items: center; margin: 4px 0 24px; }
.app-home-mark { flex: none; display: grid; place-items: center; width: 52px; height: 52px; border: 1px solid var(--border); border-radius: 14px; color: var(--accent); background: var(--bg-raised); font-family: monospace; font-size: 24px; }
.eyebrow { color: var(--text-dim); font-size: 8px; letter-spacing: .12em; }
h1 { font-size: 27px; margin: 4px 0 5px; letter-spacing: -.03em; } h1 span { color: var(--accent); }
.app-intro p { font-size: 11px; color: var(--text-dim); margin: 0; line-height: 1.6; }
.version-card { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 16px; background: var(--bg-panel); border: 1px solid var(--border); border-radius: 12px; }
h2 { margin: 5px 0 0; font-size: 13px; font-weight: 550; }
.version-value { color: var(--accent); font-size: 22px; font-family: ui-monospace, monospace; text-align: right; }
.version-value small { display: block; margin-top: 5px; color: var(--text-dim); font: 9px system-ui, sans-serif; }
.release-heading { display: flex; align-items: center; justify-content: space-between; margin: 26px 0 12px; }
.release-heading span { font-size: 8px; letter-spacing: .1em; color: var(--text-dim); }
button { font: inherit; cursor: pointer; }
button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.release-card { display: block; width: 100%; padding: 14px; margin: 0 0 9px; text-align: left; border: 1px solid var(--border); border-radius: 11px; background: var(--bg-panel); color: var(--text); }
.release-card:hover { border-color: var(--accent); }
.release-title { display: flex; align-items: center; gap: 8px; font-size: 13px; }
.latest-badge { padding: 2px 5px; border: 1px solid var(--border); border-radius: 4px; color: var(--accent); font-size: 9px; }
.release-arrow { margin-left: auto; color: var(--accent); }
.release-card ul { margin: 10px 0; padding-left: 15px; color: var(--text-dim); font-size: 11px; line-height: 1.65; }
.release-card li { margin: 4px 0; }
.read-release { display: block; margin-top: 10px; font-size: 10px; color: var(--accent); }
.release-back { padding: 8px 0; margin-bottom: 12px; background: transparent; border: 0; color: var(--accent); font-size: 12px; }
.release-document { font-size: 13px; line-height: 1.85; overflow-wrap: anywhere; }
.release-document :deep(h1) { font-size: 21px; line-height: 1.5; margin: 0 0 20px; }
.release-document :deep(h2) { margin-top: 22px; font-size: 16px; }
.release-document :deep(h3) { font-size: 14px; }
.release-document :deep(li) { margin-bottom: 10px; }
.release-document :deep(ul) { padding-left: 20px; }
.release-document :deep(a) { color: var(--accent); }
.release-document :deep(code) { padding: 2px 4px; border-radius: 4px; background: var(--bg-raised); font-size: .9em; }

.art-note { margin: 10px 3px 0; color: var(--text-dim); font-size: 11px; }

.art-note { display: flex; align-items: center; justify-content: center; gap: 5px; margin: 14px 0 0; padding: 10px 12px; border: 1px dashed color-mix(in srgb, var(--accent) 28%, transparent); border-radius: 9px; background: color-mix(in srgb, var(--accent) 5%, var(--bg)); animation: art-note-float 4s ease-in-out infinite; }
.art-note-author { color: var(--accent); font-family: ui-monospace, monospace; font-size: 11px; }
.art-note-message { color: var(--text); font-size: 11px; text-shadow: 0 0 14px color-mix(in srgb, var(--accent) 35%, transparent); }
.art-note-cursor { color: var(--accent); font-size: 12px; animation: art-cursor-blink 1.4s steps(2, start) infinite; }
.art-note-spark { margin-left: 3px; color: var(--accent); animation: art-spark-glow 3s ease-in-out infinite; }
@keyframes art-note-float { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-2px); } }
@keyframes art-cursor-blink { 0%, 100% { opacity: .85; } 50% { opacity: .15; } }
@keyframes art-spark-glow { 0%, 100% { opacity: .4; transform: scale(.85); } 50% { opacity: 1; transform: scale(1.15); } }
@media (prefers-reduced-motion: reduce) { .art-note, .art-note-cursor, .art-note-spark { animation: none; } }
</style>
