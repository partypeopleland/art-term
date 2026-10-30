<script setup lang="ts">
defineProps<{ kind: 'host' | 'agent' | 'screen' }>()
</script>
<template>
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <template v-if="kind === 'host'"><rect x="3" y="4" width="18" height="13" rx="2" /><path d="M8 21h8M12 17v4M7 8h5" /></template>
    <template v-else-if="kind === 'agent'"><path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5Z" /><path d="M19 3v4M17 5h4" /></template>
    <template v-else><rect x="3" y="5" width="18" height="14" rx="2" /><path d="M8 10h8M8 14h5" /></template>
  </svg>
</template>
<style scoped>
svg { width: 20px; height: 20px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; }
</style>
