import { createApp } from 'vue'
import Preview from './Preview.vue'
import Phone from './Phone.vue'
import '../src/style.css'

createApp(new URLSearchParams(location.search).has('phone') ? Phone : Preview).mount('#app')
