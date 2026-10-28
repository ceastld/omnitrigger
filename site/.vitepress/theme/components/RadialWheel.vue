<script setup lang="ts">
import { computed } from 'vue'

// 仿照桌面程序“编辑轨道”的四槽轮盘：正上方为第一槽，顺时针排列。
const props = defineProps<{ slots: string[]; selected?: number | null; title?: string; hint?: string }>()

const C = 120
const R_OUT = 104
const R_IN = 44
const GAP = 1.6

function point(r: number, deg: number) {
  const a = (deg * Math.PI) / 180
  return [C + r * Math.sin(a), C - r * Math.cos(a)]
}

function sector(index: number, count: number) {
  const span = 360 / count
  const start = index * span - span / 2 + GAP
  const end = index * span + span / 2 - GAP
  const large = end - start > 180 ? 1 : 0
  const [x1, y1] = point(R_OUT, start)
  const [x2, y2] = point(R_OUT, end)
  const [x3, y3] = point(R_IN, end)
  const [x4, y4] = point(R_IN, start)
  return `M${x1} ${y1}A${R_OUT} ${R_OUT} 0 ${large} 1 ${x2} ${y2}L${x3} ${y3}A${R_IN} ${R_IN} 0 ${large} 0 ${x4} ${y4}Z`
}

const sectors = computed(() =>
  props.slots.map((label, index) => {
    const [lx, ly] = point((R_OUT + R_IN) / 2, (index * 360) / props.slots.length)
    return { label, index, d: sector(index, props.slots.length), lx, ly }
  }),
)

const ticks = Array.from({ length: 48 }, (_, i) => {
  const deg = i * 7.5
  const major = i % 12 === 0
  const [x1, y1] = point(R_OUT + 6, deg)
  const [x2, y2] = point(R_OUT + (major ? 13 : 10), deg)
  return { x1, y1, x2, y2, major, deg }
})

function lit(deg: number) {
  if (props.selected == null) return false
  const diff = ((deg - (props.selected * 360) / props.slots.length + 540) % 360) - 180
  return Math.abs(diff) <= 180 / props.slots.length
}
</script>

<template>
  <svg class="wheel" viewBox="0 0 240 240" role="img" :aria-label="`${title ?? '轮盘'}：${slots.join('、')}`">
    <defs>
      <radialGradient id="ot-wheel-disc" cx="50%" cy="45%" r="60%">
        <stop offset="0" stop-color="#1d2531" />
        <stop offset="1" stop-color="#121820" />
      </radialGradient>
      <linearGradient id="ot-wheel-hot" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#52eeb0" stop-opacity=".55" />
        <stop offset="1" stop-color="#3ec8f6" stop-opacity=".35" />
      </linearGradient>
      <filter id="ot-wheel-glow" x="-30%" y="-30%" width="160%" height="160%">
        <feGaussianBlur stdDeviation="5" />
      </filter>
    </defs>
    <circle :cx="C" :cy="C" r="119" fill="#05070a" opacity=".55" />
    <circle :cx="C" :cy="C" :r="R_OUT + 14" fill="url(#ot-wheel-disc)" stroke="#2a6b52" stroke-opacity=".55" />
    <line
      v-for="t in ticks"
      :key="t.deg"
      :x1="t.x1" :y1="t.y1" :x2="t.x2" :y2="t.y2"
      :stroke="lit(t.deg) ? '#5ee8b0' : '#3b4a55'"
      :stroke-width="t.major ? 2 : 1"
      stroke-linecap="round"
    />
    <g v-for="s in sectors" :key="s.index">
      <path v-if="selected === s.index" :d="s.d" fill="#52eeb0" opacity=".35" filter="url(#ot-wheel-glow)" />
      <path
        :d="s.d"
        :fill="selected === s.index ? 'url(#ot-wheel-hot)' : '#1a212b'"
        :stroke="selected === s.index ? '#7ef3c4' : '#26303c'"
        stroke-width="1"
      />
      <text :x="s.lx" :y="s.ly - 7" class="num" :class="{ on: selected === s.index }">0{{ s.index + 1 }}</text>
      <text :x="s.lx" :y="s.ly + 10" class="label" :class="{ on: selected === s.index }">{{ s.label }}</text>
    </g>
    <circle :cx="C" :cy="C" :r="R_IN - 6" fill="#0c1016" stroke="#2a6b52" stroke-opacity=".7" />
    <text :x="C" :y="C - 6" class="hub-kicker">ORBIT</text>
    <text :x="C" :y="C + 10" class="hub-title">{{ selected == null ? (title ?? '编辑轨道') : slots[selected] }}</text>
    <text v-if="hint" :x="C" :y="C + 23" class="hub-hint">{{ hint }}</text>
  </svg>
</template>

<style scoped>
.wheel {
  display: block;
  width: 100%;
  height: auto;
  overflow: visible;
  font-family: var(--vp-font-family-base);
}
text {
  text-anchor: middle;
  dominant-baseline: middle;
  user-select: none;
}
.num {
  font: 600 8px var(--ot-mono);
  fill: #4fbf92;
}
.label {
  font-size: 12px;
  font-weight: 600;
  fill: #c9d4d0;
}
.label.on,
.num.on {
  fill: #ffffff;
}
.hub-kicker {
  font: 700 7.5px var(--ot-mono);
  letter-spacing: .12em;
  fill: #52eeb0;
}
.hub-title {
  font-size: 11px;
  font-weight: 700;
  fill: #eef4f1;
}
.hub-hint {
  font-size: 7px;
  fill: #7b8a85;
}
</style>
