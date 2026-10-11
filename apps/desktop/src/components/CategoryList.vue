<script setup lang="ts">
// Shared saved openab-pty connections, grouped and filtered only by
// backend-resolved metadata. Explicit profile/Shell actions stay unchanged.
import { computed, ref } from 'vue'
import type { RuntimeHostDto } from '../transport/types'

interface Section {
  key: string
  label: string
  profiles: RuntimeHostDto[]
}

const props = withDefaults(
  defineProps<{
    catalogLoading: boolean
    runtimeProfiles?: RuntimeHostDto[]
    runtimeProfilesError?: string
    /** Profile id whose explicit quick request is in flight. */
    runtimePending?: string | null
    /** Profile ids with a LIVE local runtime tab — local state only. */
    runtimeConnected?: Set<string>
  }>(),
  {
    runtimeProfiles: () => [],
    runtimeProfilesError: '',
    runtimePending: null,
    runtimeConnected: () => new Set<string>()
  }
)

const emit = defineEmits<{
  'runtime-open': [profile: RuntimeHostDto]
  'runtime-shell': [profile: RuntimeHostDto]
  refresh: []
  create: []
  manage: []
}>()

const filter = ref<'all' | 'host' | 'agent'>('all')
const agentProfiles = computed(() => props.runtimeProfiles.filter((p) => p.category === 'agent'))
const computerProfiles = computed(() => props.runtimeProfiles.filter((p) => p.category !== 'agent'))

// Keep the existing Agent → Host order for the all-connections view.
const sections = computed<Section[]>(() => {
  const out: Section[] = []
  if (filter.value !== 'host' && agentProfiles.value.length) out.push({ key: 'agents', label: 'Agent', profiles: agentProfiles.value })
  if (filter.value !== 'agent' && computerProfiles.value.length) out.push({ key: 'computers', label: 'Host', profiles: computerProfiles.value })
  return out
})

const emptyVisible = computed(
  () => !props.catalogLoading && !props.runtimeProfilesError && !props.runtimeProfiles.length
)
</script>

<template>
  <div class="tree">
    <div class="connection-tools">
      <label for="runtime-filter">連線</label>
      <select id="runtime-filter" v-model="filter" data-testid="runtime-category-filter"><option value="all">全部 openab-pty 連線</option><option value="host">Host</option><option value="agent">Agent</option></select>
      <div class="connection-actions"><button class="mini" data-testid="runtime-host-settings" @click="emit('create')">新增 openab-pty 連線</button><button class="mini" data-testid="runtime-host-manage" @click="emit('manage')">管理</button><button class="mini" data-testid="refresh-catalog" :disabled="catalogLoading" @click="emit('refresh')">重新整理</button></div>
    </div>
    <div v-if="catalogLoading" class="hint">載入設定中…</div>
    <div v-if="runtimeProfilesError" class="error" data-testid="runtime-profiles-error">{{ runtimeProfilesError }}</div>
    <div v-if="emptyVisible" class="hint">設定中沒有可用的連線</div>

    <p v-if="!catalogLoading && !runtimeProfilesError && runtimeProfiles.length && !sections.length" class="hint">此分類尚無連線</p>
    <section
      v-for="s in sections"
      :key="s.key"
      class="cat"
      :data-testid="`cat-${s.key}`"
    >
      <div class="host-row">
        <span class="arrow">▾</span>
        <span class="host-label">{{ s.label }}</span>
      </div>
      <div
        v-for="p in s.profiles"
        :key="`runtime-${p.id}`"
        class="target runtime"
        :data-testid="`runtime-${p.id}`"
      >
        <span
          class="dot"
          :class="{ on: runtimeConnected.has(p.id) }"
          :data-testid="`runtime-status-${p.id}`"
          :title="runtimeConnected.has(p.id) ? '已連線' : ''"
          :aria-label="runtimeConnected.has(p.id) ? `${p.name} 已連線` : undefined"
        />
        <button
          class="t-main"
          :data-runtime-label="p.name"
          :aria-label="(p.agentName || p.name) + '，' + (p.agentTool || 'openab-pty')"
          :disabled="catalogLoading || runtimePending === p.id"
          @click="!catalogLoading && runtimePending !== p.id && emit('runtime-open', p)"
        >
          <span class="t-label">{{ p.agentName || p.name }}</span>
          <span class="t-sub">{{ runtimePending === p.id ? '查詢中…' : (p.agentTool || 'openab-pty') }}</span>
        </button>
        <button
          class="mini"
          :data-testid="`runtime-shell-${p.id}`"
          :aria-label="`${p.name} 的 Shell`"
          :disabled="catalogLoading"
          title="Shell 管理"
          @click.stop="!catalogLoading && emit('runtime-shell', p)"
        >Shell</button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.connection-tools { padding: 12px; display: grid; gap: 8px; }
.connection-tools label { font-weight: 600; }
.connection-tools select { min-height: 44px; width: 100%; padding: 8px; background: var(--bg, #16171b); color: inherit; border: 1px solid var(--border, #33353d); border-radius: 4px; font: inherit; }
.connection-actions { display: flex; flex-wrap: wrap; gap: 6px; }
.connection-actions button { min-height: 36px; margin: 0; }
:is(button, select):focus-visible { outline: 2px solid var(--accent, #5b8def); outline-offset: 2px; }
@media (pointer: coarse), (max-width: 600px) { button, .connection-actions button { min-height: 44px; } .t-main { padding-block: 8px; } }

.cat {
  margin-bottom: 2px;
}

.t-main {
  background: transparent;
  border: 0;
  padding: 0;
  color: inherit;
  font: inherit;
  text-align: left;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  cursor: pointer;
}

.target.off .t-main {
  cursor: default;
}

.t-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.t-sub {
  font-size: 11px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
