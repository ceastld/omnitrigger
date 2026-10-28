<script setup lang="ts">
import type { LogEntry } from './eventLog'

defineProps<{ entries: LogEntry[]; empty: string }>()
</script>

<template>
  <ol class="log" aria-live="polite">
    <li v-if="!entries.length" class="empty">{{ empty }}</li>
    <li v-for="entry in entries" :key="entry.id" :class="entry.kind">
      <span class="tag">{{ entry.label }}</span><span>{{ entry.text }}</span>
    </li>
  </ol>
</template>

<style scoped>
.log {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 178px;
  margin: 0 0 6px;
  padding: 0;
  list-style: none;
  font-size: 13px;
  line-height: 1.5;
  color: var(--ot-muted);
}
.log li {
  display: flex;
  align-items: baseline;
  gap: 8px;
  animation: slide .25s ease-out;
}
.log li:not(:first-child) {
  opacity: .6;
}
.log .empty {
  color: var(--ot-faint);
  animation: none;
}
@keyframes slide {
  from { opacity: 0; transform: translateY(-4px); }
}
.tag {
  flex: none;
  min-width: 36px;
  padding: 1px 6px;
  border: 1px solid currentColor;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 600;
  text-align: center;
}
.buffer .tag { color: var(--ot-amber); }
.hit .tag { color: var(--ot-mint); }
.consume .tag { color: var(--ot-sky); }
.replay .tag { color: var(--ot-violet); }
.pass .tag { color: var(--ot-faint); }
.cancel .tag { color: var(--ot-rose); }
</style>
