<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
const view = ref('app')
const scenario = ref('populated')
const reload = ref(0)
const stage = ref<HTMLElement | null>(null)
const scale = ref(1)
const src = computed(() => `/?phone&view=${view.value}&scenario=${scenario.value}&reset=${reload.value}`)
let observer: ResizeObserver | undefined
onMounted(() => {
  observer = new ResizeObserver(() => {
    if (stage.value) scale.value = Math.min(1, (stage.value.clientWidth - 24) / 390, (stage.value.clientHeight - 24) / 844)
  })
  if (stage.value) observer.observe(stage.value)
})
onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
  <div class="studio">
    <header>
      <strong>art-term 手機畫面預覽</strong>
      <label>畫面 <select v-model="view"><option value="app">正式手機元件</option><option value="draft">我的草稿</option></select></label>
      <label>情境 <select v-model="scenario" :disabled="view === 'draft'"><option value="populated">示範連線</option><option value="empty">空清單</option><option value="error">讀取失敗</option></select></label>
      <button @click="reload++">重設</button>
      <span>390 × 844 · {{ Math.round(scale * 100) }}%</span>
    </header>
    <p>假資料，只供畫面討論。點連線可看終端，點 Shell 可看選單；新增與修改僅留在這次預覽。</p>
    <div ref="stage" class="stage">
      <div class="phone-space" :style="{ width: `${390 * scale}px`, height: `${844 * scale}px` }">
        <iframe :key="src" :src="src" title="手機畫面，390 × 844" :style="{ transform: `scale(${scale})` }" />
      </div>
    </div>
    <footer>直接改正式元件即可預覽；自由設計請編輯 apps/client/ui-preview/Draft.vue。儲存後自動更新。</footer>
  </div>
</template>

<style scoped>
.studio { height: 100dvh; display: flex; flex-direction: column; background: #0e1015; }
header { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; padding: 12px 18px; background: var(--bg-panel); }
label { display: flex; gap: 6px; align-items: center; }
select, button { background: var(--bg-raised); color: var(--text); border: 1px solid var(--border); border-radius: 6px; padding: 7px; }
p, footer { color: var(--text-dim); margin: 0; padding: 8px 18px; font-size: 12px; line-height: 1.5; }
.stage { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; overflow: hidden; }
.phone-space { flex: none; box-shadow: 0 0 0 2px #454955, 0 12px 50px #0008; }
iframe { display: block; width: 390px; height: 844px; border: 0; transform-origin: top left; background: var(--bg); }
@media (max-width: 500px) { header { gap: 8px; padding: 8px; } header strong { width: 100%; } p, footer { padding: 6px 8px; } }
</style>
