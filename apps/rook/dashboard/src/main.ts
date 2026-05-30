import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import './style.css'

const app = createApp(App)
const pinia = createPinia()

app.use(pinia)
app.use(router)

// Initialize theme store after Pinia is installed
import { useThemeStore } from '@/stores/theme'
const themeStore = useThemeStore()
themeStore.init()

app.mount('#app')