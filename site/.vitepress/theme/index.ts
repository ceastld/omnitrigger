// 使用系统字体，不随站点分发 Inter 字体文件
import DefaultTheme from 'vitepress/theme-without-fonts'
import type { Theme } from 'vitepress'
import HomePage from './components/HomePage.vue'
import Mermaid from './components/Mermaid.vue'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('HomePage', HomePage)
    app.component('Mermaid', Mermaid)
  },
} satisfies Theme
