<script setup lang="ts">
import { ref } from 'vue'
const selected = ref(false)
</script>

<template>
  <main class="draft">
    <header><strong>我的畫面草稿</strong><span>手機預覽</span></header>
    <section>
      <h1>從這裡開始調整</h1>
      <p>編輯 ui-preview/Draft.vue，儲存後會立即更新。這份草稿獨立於正式 App。</p>
      <button @click="selected = !selected">{{ selected ? '已選取' : '試試這個按鈕' }}</button>
    </section>
  </main>
</template>

<style scoped>
.draft { min-height: 100dvh; padding: 20px; background: var(--bg); }
header { display: flex; justify-content: space-between; align-items: center; }
header span, p { color: var(--text-dim); line-height: 1.7; }
section { margin-top: 32px; padding: 20px; border: 1px solid var(--border); border-radius: 16px; background: var(--bg-panel); }
h1 { font-size: 22px; }
button { padding: 12px 20px; background: var(--accent); color: #10151f; border: 0; border-radius: 8px; font-size: 16px; }
</style>
