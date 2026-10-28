<script setup lang="ts">
import Keycap from './Keycap.vue'

defineProps<{ text: string; combo?: string[]; duration?: number }>()
</script>

<template>
  <div class="toast" role="status" :style="{ '--d': `${duration ?? 2600}ms` }">
    <div class="head"><span class="dot" />OmniTrigger</div>
    <div class="text">{{ text }}</div>
    <div v-if="combo?.length" class="keys">
      <template v-for="(k, i) in combo" :key="k">
        <span v-if="i" class="plus">+</span><Keycap size="sm" accent>{{ k }}</Keycap>
      </template>
    </div>
    <div class="bar" />
  </div>
</template>

<style scoped>
.toast {
  position: relative;
  width: 260px;
  max-width: 100%;
  padding: 12px 14px 14px;
  border: 1px solid var(--ot-line-strong);
  border-radius: 12px;
  background: color-mix(in srgb, var(--ot-surface-2) 94%, transparent);
  box-shadow: var(--ot-shadow);
  overflow: hidden;
  text-align: left;
}
.head {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--ot-faint);
  letter-spacing: .02em;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--ot-mint);
  box-shadow: 0 0 8px var(--ot-mint);
}
.text {
  margin: 4px 0 2px;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.45;
  color: var(--ot-ink);
}
.keys {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: 6px;
}
.plus {
  color: var(--ot-faint);
  font-size: 12px;
}
.bar {
  position: absolute;
  left: 0;
  bottom: 0;
  height: 3px;
  width: 100%;
  background: var(--ot-grad);
  transform-origin: left;
  animation: drain var(--d) linear forwards;
}
@keyframes drain {
  to { transform: scaleX(0); }
}
</style>
