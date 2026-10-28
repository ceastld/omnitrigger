<script setup lang="ts">
import { useData } from 'vitepress'
import { computed, onMounted, ref, watch } from 'vue'

const props = defineProps<{ code: string }>()
const { isDark } = useData()
const source = computed(() => decodeURIComponent(props.code))
const svg = ref('')
const failed = ref(false)
let seq = 0

async function render() {
  const id = `ot-mermaid-${Math.random().toString(36).slice(2)}-${++seq}`
  try {
    const { default: mermaid } = await import('mermaid')
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      theme: 'base',
      fontFamily: 'var(--vp-font-family-base)',
      themeVariables: isDark.value
        ? {
            darkMode: true,
            background: '#0d1217',
            primaryColor: '#14231d',
            primaryBorderColor: '#3fd99c',
            primaryTextColor: '#e8f2ee',
            lineColor: '#5ee8b0',
            secondaryColor: '#121a24',
            tertiaryColor: '#0d1217',
          }
        : {
            primaryColor: '#e6f5ee',
            primaryBorderColor: '#0e9466',
            primaryTextColor: '#0d1714',
            lineColor: '#0e8f63',
            secondaryColor: '#eef6fa',
            tertiaryColor: '#ffffff',
          },
    })
    const rendered = (await mermaid.render(id, source.value)).svg
    // 横向流程图按原始宽度显示，窄屏时横向滚动，避免整体缩小到无法阅读
    const natural = rendered.match(/max-width:\s*([\d.]+)px/)?.[1]
    svg.value = natural ? rendered.replace('width="100%"', `width="${Math.ceil(Number(natural))}"`) : rendered
    failed.value = false
  } catch {
    failed.value = true
  }
}

onMounted(render)
watch(isDark, render)
</script>

<template>
  <div class="ot-mermaid">
    <div v-if="svg && !failed" class="ot-mermaid-svg" v-html="svg" />
    <pre v-else><code>{{ source }}</code></pre>
  </div>
</template>

<style scoped>
.ot-mermaid {
  margin: 16px 0;
  padding: 18px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 12px;
  background: var(--vp-c-bg-soft);
  overflow-x: auto;
}
.ot-mermaid-svg :deep(svg) {
  display: block;
  margin: 0 auto;
  height: auto;
}
.ot-mermaid pre {
  margin: 0;
  font-size: 13px;
  white-space: pre;
}
</style>
