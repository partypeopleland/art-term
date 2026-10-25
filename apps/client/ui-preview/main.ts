import { createApp } from 'vue'
import Preview from './Preview.vue'
import Phone from './Phone.vue'
import Desktop from './Desktop.vue'
import '../src/shared/style.css'

const params = new URLSearchParams(location.search)
createApp(params.has('desktop') ? Desktop : params.has('phone') ? Phone : Preview).mount('#app')
