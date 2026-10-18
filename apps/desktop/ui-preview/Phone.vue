<script setup lang="ts">
import MobileApp from '../src/mobile/MobileApp.vue'
import { createMobileXtermTerminal } from '../src/terminal/handle'
import Draft from './Draft.vue'
import { createPreviewTransport, type Scenario } from './transport'
import { createPreviewVoicePlugin } from './voice'

const params = new URLSearchParams(location.search)
const scenario = params.get('scenario') as Scenario
const transport = createPreviewTransport(scenario)
const terminalFactory = (el: HTMLElement) => createMobileXtermTerminal(el)
const voicePlugin = createPreviewVoicePlugin()
</script>

<template>
  <Draft v-if="params.get('view') === 'draft'" />
  <MobileApp v-else :transport="transport" :terminal-factory="terminalFactory" :native-history-factory="null" :voice-plugin="voicePlugin" />
</template>
