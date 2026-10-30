<script setup lang="ts">
import { computed, ref } from 'vue'
import HostConnections from './HostConnections.vue'
import type { InstanceDesktop, RuntimeHostDto, HostDto, TargetDto, Transport } from '../transport/types'

const props = withDefaults(defineProps<{
  screens?: InstanceDesktop[]
  screensError?: string
  transport: Transport
  active: boolean
  catalogLoading: boolean
  runtimeProfiles: RuntimeHostDto[]
  runtimeProfilesError: string
  runtimePending: string | null
  /** Only a live local tab lights the connection indicator. */
  runtimeConnected: Set<string>
}>(), { screens: () => [], screensError: '' })
const emit = defineEmits<{
  open: [host: HostDto, target: TargetDto]
  create: [category: 'host' | 'agent']
}>()
const collapsed = ref<Record<string, boolean>>({})
function toggleCategory(key: string): void { collapsed.value[key] = !collapsed.value[key] }
const sections = computed(() => [
  { key: 'computers', title: 'Computers', subtitle: '主機連線', profiles: props.runtimeProfiles.filter(p => p.category !== 'agent') },
  { key: 'agents', title: 'Agents', subtitle: '工作助理', profiles: props.runtimeProfiles.filter(p => p.category === 'agent') }
])
</script>

<template>
  <div class="dashboard">
    <div v-if="runtimeProfilesError" class="dashboard-error" data-testid="runtime-profiles-error" role="alert">{{ runtimeProfilesError }}</div>
    <p v-if="catalogLoading" class="loading" role="status">載入設定中…</p>

    <section v-for="section in sections" :key="section.key" class="connection-section" :class="section.key" :data-testid="`cat-${section.key}`" :aria-labelledby="`heading-${section.key}`">
      <header class="section-heading">
        <button class="category-toggle" :aria-expanded="!collapsed[section.key]" :aria-controls="`category-content-${section.key}`" @click="toggleCategory(section.key)">
        <span class="section-icon">
          <svg v-if="section.key === 'computers'" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="13" rx="2" /><path d="M8 21h8M12 17v4M7 8h5" /></svg>
          <svg v-else viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5Z" /><path d="M19 3v4M17 5h4" /></svg>
        </span>
        <div class="section-titles"><h2 :id="`heading-${section.key}`">{{ section.title }}</h2><span>{{ section.subtitle }}</span></div>
        <span class="section-count" :aria-label="`${section.profiles.length} 個設定`">{{ section.profiles.length.toString().padStart(2, '0') }}</span>
        </button>
      </header>
      <div v-show="!collapsed[section.key]" :id="`category-content-${section.key}`" class="section-content">
        <HostConnections v-for="profile in section.profiles" :key="`${profile.id}:${profile.revision}`" :profile="profile" :transport="transport" :active="active" :connected="runtimeConnected.has(profile.id)" @open="(host, target) => emit('open', host, target)" />
        <div v-if="!section.profiles.length" class="empty-section">
          <span class="empty-line" aria-hidden="true" />
          <span>{{ catalogLoading ? '正在讀取連線' : runtimeProfilesError ? '設定暫時無法讀取' : section.key === 'computers' ? '尚未新增主機連線' : '尚未新增工作助理' }}</span>
        </div>
      </div>
    </section>

    <section class="connection-section screens" data-testid="cat-screens" aria-labelledby="heading-screens">
      <header class="section-heading">
        <button class="category-toggle" :aria-expanded="!collapsed.screens" aria-controls="category-content-screens" @click="toggleCategory('screens')">
        <span class="section-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="2" /><path d="M8 10h8M8 14h5" /></svg></span>
        <div class="section-titles"><h2 id="heading-screens">Screens</h2><span>畫面工作區</span></div>
        <span class="section-count" :aria-label="`${screens.length} 個設定`">{{ screens.length.toString().padStart(2, '0') }}</span>
        </button>
      </header>
      <div v-show="!collapsed.screens" id="category-content-screens" class="section-content">
        <p v-if="screensError" class="dashboard-error" role="alert">{{ screensError }}</p>
        <div v-for="screen in screens" :key="screen.id" class="saved-screen-row" :data-testid="`screen-${screen.id}`"><span class="screen-status-dot" /><div><strong>{{ screen.name }}</strong><span>{{ screen.mcpUrl }}</span></div></div>
        <div v-if="!screens.length && !screensError" class="empty-section"><span class="empty-line" aria-hidden="true" /><span>尚未新增 Screen，請到齒輪的連線設定新增</span></div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.dashboard { --dash-text: var(--text); --dash-muted: var(--text-dim); display: flex; flex-direction: column; gap: 9px; color: var(--dash-text); }
svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; flex: none; }
button { font: inherit; cursor: pointer; touch-action: manipulation; }
button:disabled { opacity: .45; cursor: default; }
button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
.dashboard-tools { display: flex; align-items: center; gap: 8px; min-width: 0; }
.tools-label { color: var(--text-dim); font-size: 11px; letter-spacing: .06em; flex: 1; }
.tool-actions { display: flex; align-items: center; gap: 4px; }
.tool { min-height: 44px; min-width: 44px; display: inline-flex; align-items: center; justify-content: center; gap: 4px; padding: 0 10px; border: 1px solid transparent; border-radius: 9px; color: var(--text-dim); background: transparent; font-size: 12px; }
.tool.add { color: var(--accent); background: var(--bg-raised); border-color: var(--border); }
.tool:hover { background: var(--bg-hover); color: var(--text); }
.refresh { padding: 0; }
.connection-section { --section-accent: rgb(var(--computer-rgb)); --section-rgb: var(--computer-rgb); min-width: 0; overflow: hidden; border: 1px solid var(--border); border-radius: 11px; background: linear-gradient(115deg, rgba(var(--section-rgb), .035), transparent 75%), var(--bg-panel); box-shadow: 0 4px 14px #00000012; }
.agents { --section-accent: rgb(var(--agent-rgb)); --section-rgb: var(--agent-rgb); }
.screens { --section-accent: rgb(var(--screen-rgb)); --section-rgb: var(--screen-rgb); }
.section-heading { display: flex; align-items: center; gap: 9px; padding: 9px 12px; }
.section-icon { display: grid; place-items: center; width: 28px; height: 28px; flex: none; color: var(--section-accent); background: rgba(var(--section-rgb), .09); border: 1px solid rgba(var(--section-rgb), .13); border-radius: 9px; }
.section-titles { min-width: 0; flex: 1; }
.section-titles h2 { margin: 0 0 2px; font-size: 12px; font-weight: 650; letter-spacing: .025em; }
.section-titles > span { color: var(--dash-muted); font-size: 10px; }
.section-count { color: var(--section-accent); font-family: ui-monospace, monospace; font-size: 11px; padding: 4px 6px; border: 1px solid rgba(var(--section-rgb), .18); background: rgba(var(--section-rgb), .04); border-radius: 5px; }
.section-content { margin: 0 11px 6px; }
.connection-row { display: flex; align-items: stretch; border-top: 1px solid var(--border); min-width: 0; }
.connection-main { display: flex; align-items: center; gap: 9px; flex: 1; min-width: 0; min-height: 46px; padding: 5px 3px; border: 0; background: transparent; text-align: left; color: inherit; border-radius: 6px; }
.connection-main:hover { background: rgba(var(--section-rgb), .045); }
.connection-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.connection-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 12px; font-weight: 550; }
.connection-sub { color: var(--text-dim); font-size: 10px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.connection-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--dot-idle); margin: 0 3px; flex: none; }
.connection-dot.on { background: var(--ok); box-shadow: 0 0 8px color-mix(in srgb, var(--ok) 40%, transparent); }
.shell-action { display: flex; gap: 4px; align-items: center; justify-content: center; min-width: 58px; min-height: 46px; margin: 0; padding: 0 4px 0 10px; color: var(--text-dim); background: transparent; border: 0; border-left: 1px solid var(--border); border-radius: 0 6px 6px 0; font-size: 10px; }
.shell-action svg { width: 12px; height: 12px; }
.shell-action:hover { color: var(--section-accent); background: rgba(var(--section-rgb), .05); }
.empty-section { display: flex; gap: 9px; align-items: center; border-top: 1px solid var(--border); min-height: 38px; padding: 7px 4px; color: var(--text-dim); font-size: 11px; }
.empty-line { width: 15px; height: 1px; background: var(--border); }
.future-badge { padding: 4px 7px; color: var(--text-dim); background: var(--bg-raised); border: 1px solid var(--border); border-radius: 5px; font-size: 10px; }
.screen-placeholder { display: flex; align-items: center; gap: 10px; min-height: 56px; margin: 0 11px 8px; padding: 9px; border: 1px dashed var(--border); border-radius: 8px; background: var(--bg); }
.screen-placeholder strong { color: var(--text); font-weight: 450; font-size: 11px; }
.screen-placeholder p { margin: 4px 0 0; color: var(--text-dim); font-size: 10px; }
.screen-grid { display: grid; grid-template-columns: repeat(2, 8px); gap: 3px; padding: 3px; flex: none; opacity: .65; }
.screen-grid i { width: 8px; height: 8px; border: 1px solid var(--section-accent); border-radius: 2px; }
.placeholder-plus { margin-left: auto; color: var(--text-dim); font-size: 18px; }
.dashboard-footnote { display: flex; align-items: center; justify-content: center; gap: 6px; margin: 1px 0; color: var(--text-dim); font-size: 10px; }
.dashboard-footnote > span { width: 4px; height: 4px; border-radius: 50%; background: var(--ok); opacity: .6; }
.dashboard-error { padding: 10px 12px; border: 1px solid var(--err); border-radius: 8px; color: var(--err); background: var(--bg-raised); font-size: 12px; line-height: 1.5; overflow-wrap: anywhere; }
.loading { margin: 0; padding: 0 4px; font-size: 11px; color: var(--text-dim); }
.spinning { animation: rotate 1.4s linear infinite; }
@keyframes rotate { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .spinning { animation: none; } }
@media (max-width: 340px) { .section-heading { padding-inline: 11px; } .tools-label { font-size: 10px; letter-spacing: 0; } .tool { padding-inline: 8px; } .refresh { padding: 0; } }

.section-heading { gap: 8px; padding-block: 5px; }
.section-titles { flex: none; }
.section-count { flex: none; margin-left: 1px; }
.section-actions { margin-left: auto; display: flex; align-items: center; gap: 2px; }
.section-action { min-height: 36px; min-width: 36px; padding: 0 6px; border: 0; border-radius: 7px; background: transparent; color: var(--text-dim); font: inherit; font-size: 11px; cursor: pointer; }
.add-action { color: var(--section-accent); }
.section-action:hover:not(:disabled) { background: var(--bg-raised); color: var(--section-accent); }
.section-action:disabled { opacity: .4; cursor: default; }
@media (max-width: 340px) { .section-heading { gap: 6px; padding-inline: 9px; } .section-icon { width: 25px; height: 25px; } .section-action { min-width: 32px; padding-inline: 4px; } .section-count { padding-inline: 4px; } }

.category-toggle { display: flex; align-items: center; gap: 8px; flex: 1; min-width: 0; min-height: 36px; padding: 0; border: 0; border-radius: 6px; background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; }
.category-toggle:hover { background: rgba(var(--section-rgb), .04); }
.category-toggle:focus-visible { outline: 2px solid var(--section-accent); outline-offset: 2px; }
.category-toggle .section-titles { display: block; }
.category-toggle .section-titles h2 { display: block; }
.category-toggle[aria-expanded="false"] .section-icon { opacity: .65; }
@media (max-width: 340px) { .category-toggle { gap: 6px; } }

.category-toggle .section-count { margin-left: auto; }
.saved-screen-row { display: flex; align-items: center; gap: 10px; padding: 8px 3px; border-top: 1px solid var(--border); }
.saved-screen-row > div { min-width: 0; display: flex; flex-direction: column; gap: 4px; }
.saved-screen-row strong { font-size: 12px; font-weight: 550; }
.saved-screen-row > div > span { font-size: 10px; color: var(--text-dim); overflow-wrap: anywhere; }
.screen-status-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--dot-idle); flex: none; }
</style>
