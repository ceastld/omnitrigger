<script setup lang="ts">
defineProps<{ pressed?: boolean; size?: 'sm' | 'md' | 'lg'; accent?: boolean }>()
</script>

<template>
  <kbd class="ot-key" :class="[size ?? 'md', { pressed, accent }]"><slot /></kbd>
</template>

<style scoped>
.ot-key {
  --h: 34px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-width: var(--h);
  height: var(--h);
  padding: 0 12px;
  border: 1px solid var(--ot-key-edge);
  border-radius: 9px;
  background: linear-gradient(180deg, var(--ot-key-top), var(--ot-key-bottom));
  box-shadow:
    0 3px 0 0 color-mix(in srgb, var(--ot-key-edge) 70%, #000),
    0 6px 16px -8px rgba(0, 0, 0, .6),
    inset 0 1px 0 rgba(255, 255, 255, .08);
  color: var(--ot-key-ink);
  font: 600 14px/1 var(--vp-font-family-base);
  white-space: nowrap;
  transform: translateY(0);
  transition: transform .09s ease, box-shadow .09s ease, border-color .15s ease, color .15s ease;
}
.ot-key.sm {
  --h: 26px;
  padding: 0 8px;
  border-radius: 7px;
  font-size: 12px;
}
.ot-key.lg {
  --h: 52px;
  padding: 0 20px;
  border-radius: 13px;
  font-size: 19px;
}
.ot-key.accent {
  border-color: var(--ot-line-strong);
  color: var(--ot-mint);
}
.ot-key.pressed {
  transform: translateY(3px);
  border-color: var(--ot-mint);
  color: var(--ot-mint);
  box-shadow:
    0 0 0 0 transparent,
    0 0 22px -4px color-mix(in srgb, var(--ot-mint) 60%, transparent),
    inset 0 1px 0 rgba(255, 255, 255, .06);
}
</style>
