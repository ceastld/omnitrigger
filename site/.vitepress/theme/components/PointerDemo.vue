<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref } from 'vue'
import DemoLog from './DemoLog.vue'
import DemoToast from './DemoToast.vue'
import RadialWheel from './RadialWheel.vue'
import { useEventLog } from './eventLog'

// 网页模拟：参数取自默认预设（右键快拖 18px / <180ms 与静止长按 300ms 共存），
// 按 CSS 像素计算；桌面程序使用物理像素与系统 Hook。
const DRAG_PX = 18
const DRAG_MS = 180
const HOLD_MS = 300
const HOLD_MOVE_PX = 8
const CANCEL_PX = DRAG_PX / 2
const TIMELINE_MS = 400

type Phase = 'idle' | 'pending' | 'drag' | 'hold' | 'replayed'

const slots = ['复制', '粘贴', '全选', '撤销']
const cards = [
  { name: '截图', kind: '插件动作', icon: '◎' },
  { name: '最大化 / 还原', kind: '窗口操作', icon: '▢' },
  { name: '音量 +5', kind: '系统调节', icon: '◢' },
]

const pad = ref<HTMLElement | null>(null)
const phase = ref<Phase>('idle')
const start = reactive({ x: 0, y: 0, t: 0 })
const cur = reactive({ x: 0, y: 0 })
const elapsed = ref(0)
const selected = ref<number | null>(null)
const panelAt = ref<{ x: number; y: number } | null>(null)
const menuAt = ref<{ x: number; y: number } | null>(null)
const toast = ref<{ id: number; text: string } | null>(null)
const { entries, push } = useEventLog()

let holdTimer: ReturnType<typeof setTimeout> | undefined
let toastTimer: ReturnType<typeof setTimeout> | undefined
let frame = 0
let pointerId = -1

const dist = computed(() => Math.hypot(cur.x - start.x, cur.y - start.y))
const pressing = computed(() => phase.value === 'pending' || phase.value === 'drag' || phase.value === 'hold')

function local(event: PointerEvent) {
  const rect = pad.value!.getBoundingClientRect()
  return { x: event.clientX - rect.left, y: event.clientY - rect.top }
}

function showToast(text: string) {
  clearTimeout(toastTimer)
  toast.value = { id: Date.now(), text }
  toastTimer = setTimeout(() => (toast.value = null), 2400)
}

function tick() {
  elapsed.value = performance.now() - start.t
  if (phase.value === 'pending' || phase.value === 'drag') frame = requestAnimationFrame(tick)
}

function slotFor(dx: number, dy: number) {
  if (Math.hypot(dx, dy) <= CANCEL_PX) return null
  const deg = ((Math.atan2(dx, -dy) * 180) / Math.PI + 360) % 360
  return Math.round(deg / (360 / slots.length)) % slots.length
}

function closePanel(reason?: string) {
  if (!panelAt.value) return
  panelAt.value = null
  if (reason) push('cancel', reason)
}

function onDown(event: PointerEvent) {
  const p = local(event)
  pad.value!.focus({ preventScroll: true })
  menuAt.value = null
  if (panelAt.value) {
    if (!(event.target as HTMLElement).closest('.mini-panel')) closePanel('点击面板以外 → 收起面板')
    return
  }
  if (phase.value !== 'idle') return
  const rightLike = event.pointerType === 'mouse' ? event.button === 2 : event.isPrimary
  if (!rightLike) {
    if (event.button === 0) push('pass', '左键按下 → 本演示未绑定，透传')
    return
  }
  event.preventDefault()
  pad.value!.setPointerCapture(event.pointerId)
  pointerId = event.pointerId
  Object.assign(start, { ...p, t: performance.now() })
  Object.assign(cur, p)
  elapsed.value = 0
  selected.value = null
  phase.value = 'pending'
  push('buffer', '右键按下 → 暂存，判断短按、快拖或长按')
  cancelAnimationFrame(frame)
  frame = requestAnimationFrame(tick)
  holdTimer = setTimeout(onHoldTimer, HOLD_MS)
}

function onHoldTimer() {
  if (phase.value !== 'pending') return
  if (dist.value > HOLD_MOVE_PX) {
    replay(`位移超过 ${HOLD_MOVE_PX}px → 取消长按`)
    return
  }
  phase.value = 'hold'
  elapsed.value = HOLD_MS
  panelAt.value = { x: cur.x, y: cur.y }
  push('hit', `静止长按 ${HOLD_MS}ms → 在指针处显示触发面板`)
}

function replay(reason: string) {
  clearTimeout(holdTimer)
  phase.value = 'replayed'
  push('cancel', reason)
  push('replay', '回放右键按下，后续输入照常交付')
}

function onMove(event: PointerEvent) {
  if (event.pointerId !== pointerId) return
  Object.assign(cur, local(event))
  const t = performance.now() - start.t
  if (phase.value === 'pending') {
    if (t < DRAG_MS && dist.value >= DRAG_PX) {
      clearTimeout(holdTimer)
      phase.value = 'drag'
      push('hit', `${Math.round(t)}ms 内移动 ${Math.round(dist.value)}px → 快拖命中，打开轮盘`)
    } else if (t >= DRAG_MS && dist.value > HOLD_MOVE_PX) {
      replay(`快拖时窗已过且位移超过 ${HOLD_MOVE_PX}px → 取消长按`)
    }
  }
  if (phase.value === 'drag') selected.value = slotFor(cur.x - start.x, cur.y - start.y)
}

function onUp(event: PointerEvent) {
  if (event.pointerId !== pointerId) return
  pointerId = -1
  clearTimeout(holdTimer)
  cancelAnimationFrame(frame)
  const p = local(event)
  switch (phase.value) {
    case 'pending':
      push('replay', '短按松开 → 回放右键按下与释放')
      menuAt.value = p
      break
    case 'drag': {
      const slot = slotFor(p.x - start.x, p.y - start.y)
      if (slot === null) push('cancel', `回到起点 ${CANCEL_PX}px 内松开 → 取消选择`)
      else {
        push('consume', `松开执行「${slots[slot]}」，整次按压已消费`)
        showToast(`轮盘 · 编辑轨道：${slots[slot]}（演示）`)
      }
      selected.value = null
      break
    }
    case 'hold':
      push('consume', '右键释放 → 随整次按压一起消费')
      break
    case 'replayed':
      push('pass', '右键释放 → 透传，原右键菜单照常出现')
      menuAt.value = p
      break
  }
  phase.value = 'idle'
}

function onCancel() {
  clearTimeout(holdTimer)
  cancelAnimationFrame(frame)
  if (phase.value === 'drag') push('cancel', '指针被系统取消 → 取消轮盘')
  phase.value = 'idle'
  selected.value = null
  pointerId = -1
}

function runCard(index: number) {
  const card = cards[index]
  panelAt.value = null
  push('hit', `面板执行「${card.name}」→ 先收起再执行`)
  showToast(`${card.kind} · ${card.name}（演示）`)
}

function onKeydown(event: KeyboardEvent) {
  if (!panelAt.value) return
  if (event.key === 'Escape') {
    event.preventDefault()
    closePanel('Esc → 收起面板')
  } else if (/^[1-3]$/.test(event.key)) {
    event.preventDefault()
    runCard(Number(event.key) - 1)
  }
}

const panelStyle = computed(() => {
  if (!panelAt.value || !pad.value) return {}
  const w = Math.min(300, pad.value.clientWidth - 16)
  const x = Math.min(Math.max(panelAt.value.x - w / 2, 8), pad.value.clientWidth - w - 8)
  const y = Math.min(Math.max(panelAt.value.y - 70, 8), pad.value.clientHeight - 150)
  return { left: `${x}px`, top: `${y}px`, width: `${w}px` }
})

const progress = computed(() => Math.min(elapsed.value, TIMELINE_MS) / TIMELINE_MS)
const holdRatio = computed(() => Math.min(elapsed.value / HOLD_MS, 1))

onBeforeUnmount(() => {
  clearTimeout(holdTimer)
  clearTimeout(toastTimer)
  cancelAnimationFrame(frame)
})
</script>

<template>
  <div class="pd">
    <div class="pd-main">
      <div
        ref="pad"
        class="pad"
        :class="{ active: pressing }"
        tabindex="0"
        role="application"
        aria-label="鼠标手势演示：在此区域按住右键"
        @pointerdown="onDown"
        @pointermove="onMove"
        @pointerup="onUp"
        @pointercancel="onCancel"
        @contextmenu.prevent
        @keydown="onKeydown"
      >
        <div class="pad-hint" :class="{ hidden: phase !== 'idle' || panelAt || menuAt }">
          <strong>在这里使用右键</strong>
          <span>短按 · 静止按住 0.3 秒 · 按下后快速拖动</span>
          <small>触屏可直接按住或拖动</small>
        </div>

        <svg v-if="phase === 'pending' || phase === 'replayed'" class="probe" :style="{ left: `${start.x}px`, top: `${start.y}px` }" viewBox="-40 -40 80 80">
          <circle r="18" class="ring-drag" />
          <circle r="8" class="ring-hold" />
          <circle r="26" class="ring-progress" pathLength="100" :style="{ strokeDashoffset: 100 - holdRatio * 100 }" />
          <line :x2="cur.x - start.x" :y2="cur.y - start.y" class="vector" />
        </svg>

        <div v-if="phase === 'drag'" class="wheel-wrap" :style="{ left: `${start.x}px`, top: `${start.y}px` }">
          <RadialWheel :slots="slots" :selected="selected" hint="松开执行 · 回中取消" />
          <svg class="vector-layer" viewBox="-130 -130 260 260">
            <circle r="9" class="cancel-zone" />
            <line :x2="Math.max(-125, Math.min(125, cur.x - start.x))" :y2="Math.max(-125, Math.min(125, cur.y - start.y))" class="vector" />
          </svg>
        </div>

        <div v-if="panelAt" class="mini-panel" :style="panelStyle" @pointerdown.stop>
          <div class="mp-head">
            <span class="mp-search">筛选动作</span>
            <span class="mp-live">● 监听中</span>
          </div>
          <div class="mp-cards">
            <button v-for="(card, i) in cards" :key="card.name" type="button" class="mp-card" @click="runCard(i)">
              <span class="mp-icon">{{ card.icon }}</span>
              <span class="mp-name">{{ card.name }}</span>
              <span class="mp-num">{{ i + 1 }}</span>
            </button>
          </div>
          <div class="mp-foot">点击运行 · 数字键直达 · Esc 关闭</div>
        </div>

        <div v-if="menuAt" class="ctx" :style="{ left: `${Math.min(menuAt.x, (pad?.clientWidth ?? 300) - 170)}px`, top: `${Math.min(menuAt.y, (pad?.clientHeight ?? 300) - 120)}px` }">
          <div class="ctx-title">原右键菜单（已回放）</div>
          <i /><i /><i />
        </div>

        <transition name="toast">
          <DemoToast v-if="toast" :key="toast.id" class="toast" :text="toast.text" :combo="['右键']" :duration="2400" />
        </transition>
      </div>

      <div class="timeline" aria-hidden="true">
        <div class="track">
          <div class="fill" :class="phase" :style="{ transform: `scaleX(${progress})` }" />
          <span class="mark" :style="{ left: `${(DRAG_MS / TIMELINE_MS) * 100}%` }"><em>180ms 快拖时窗</em></span>
          <span class="mark hold" :style="{ left: `${(HOLD_MS / TIMELINE_MS) * 100}%` }"><em>300ms 长按</em></span>
        </div>
        <div class="readout">
          <span>{{ Math.round(Math.min(elapsed, 9999)) }}ms</span>
          <span>位移 {{ Math.round(dist) }}px</span>
        </div>
      </div>
    </div>

    <aside class="pd-side">
      <div class="side-title">引擎决策</div>
      <DemoLog :entries="entries" empty="按下右键后，这里显示暂存、命中、取消与回放。" />
      <div class="side-title">同一个右键，三种结果</div>
      <ul class="rules">
        <li><b class="c-replay">短按</b>松开时回放原右键，菜单照常出现</li>
        <li><b class="c-hit">快拖</b>180ms 内移动 18px，打开轮盘，松开执行</li>
        <li><b class="c-sky">长按</b>静止 300ms，在指针处显示触发面板</li>
      </ul>
    </aside>
  </div>
</template>

<style scoped>
.pd {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(0, 1fr);
  gap: 18px;
}
.pd-main {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
}
.pad {
  position: relative;
  height: 340px;
  border: 1px solid var(--ot-line);
  border-radius: 14px;
  background:
    radial-gradient(circle at 1px 1px, color-mix(in srgb, var(--ot-ink) 10%, transparent) 1px, transparent 0) 0 0 / 22px 22px,
    var(--ot-surface);
  overflow: hidden;
  touch-action: none;
  user-select: none;
  -webkit-user-select: none;
  -webkit-touch-callout: none;
  cursor: crosshair;
  outline: none;
  transition: border-color .2s;
}
.pad.active,
.pad:focus-visible {
  border-color: var(--ot-line-strong);
}
.pad-hint {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
  pointer-events: none;
  transition: opacity .2s;
}
.pad-hint.hidden {
  opacity: 0;
}
.pad-hint strong {
  font-size: 17px;
  color: var(--ot-ink);
}
.pad-hint span {
  font-size: 14px;
  color: var(--ot-muted);
}
.pad-hint small {
  font-size: 12px;
  color: var(--ot-faint);
}
.probe {
  position: absolute;
  width: 80px;
  height: 80px;
  margin: -40px 0 0 -40px;
  overflow: visible;
  pointer-events: none;
}
.ring-drag {
  fill: none;
  stroke: var(--ot-sky);
  stroke-dasharray: 3 3;
  opacity: .7;
}
.ring-hold {
  fill: color-mix(in srgb, var(--ot-amber) 18%, transparent);
  stroke: var(--ot-amber);
  opacity: .8;
}
.ring-progress {
  fill: none;
  stroke: var(--ot-mint);
  stroke-width: 3;
  stroke-linecap: round;
  stroke-dasharray: 100;
  transform: rotate(-90deg);
}
.vector {
  stroke: var(--ot-mint);
  stroke-width: 1.5;
  stroke-linecap: round;
}
.wheel-wrap {
  position: absolute;
  width: 236px;
  height: 236px;
  margin: -118px 0 0 -118px;
  pointer-events: none;
  animation: pop .14s cubic-bezier(.2, .9, .3, 1.2);
}
.vector-layer {
  position: absolute;
  inset: 0;
  overflow: visible;
}
.cancel-zone {
  fill: color-mix(in srgb, var(--ot-rose) 22%, transparent);
  stroke: var(--ot-rose);
  stroke-dasharray: 2 2;
}
@keyframes pop {
  from { opacity: 0; transform: scale(.86); }
}
.mini-panel {
  position: absolute;
  padding: 10px;
  border: 1px solid var(--ot-line-strong);
  border-radius: 14px;
  background: #10141a;
  box-shadow: 0 24px 60px -20px rgba(0, 0, 0, .8);
  color: #e8f2ee;
  cursor: default;
  animation: pop .14s ease-out;
}
.mp-head {
  display: flex;
  gap: 8px;
  margin-bottom: 8px;
  font-size: 11px;
}
.mp-search {
  flex: 1;
  padding: 5px 9px;
  border: 1px solid #262e38;
  border-radius: 8px;
  color: #6b7873;
}
.mp-live {
  align-self: center;
  padding: 3px 8px;
  border: 1px solid #1f5a43;
  border-radius: 999px;
  color: #5ee8b0;
}
.mp-cards {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 6px;
}
.mp-card {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  min-height: 70px;
  padding: 8px;
  border: 1px solid #222a33;
  border-radius: 10px;
  background: #161c24;
  color: #e8f2ee;
  font-size: 12px;
  font-weight: 600;
  text-align: left;
  cursor: pointer;
  transition: border-color .15s, background .15s;
}
.mp-card:hover {
  border-color: #2f8f69;
  background: #18241f;
}
.mp-icon {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border-radius: 6px;
  background: #1d2a25;
  color: #5ee8b0;
  font-size: 11px;
}
.mp-num {
  position: absolute;
  top: 7px;
  right: 7px;
  padding: 0 5px;
  border: 1px solid #2b333d;
  border-radius: 5px;
  font: 10px var(--ot-mono);
  color: #7b8a85;
}
.mp-foot {
  margin-top: 8px;
  font-size: 10.5px;
  color: #6b7873;
}
.ctx {
  position: absolute;
  display: flex;
  flex-direction: column;
  gap: 7px;
  width: 160px;
  padding: 10px 12px 12px;
  border: 1px solid var(--ot-line);
  border-radius: 10px;
  background: var(--ot-surface-2);
  box-shadow: var(--ot-shadow);
  pointer-events: none;
  animation: pop .12s ease-out;
}
.ctx-title {
  font-size: 11px;
  color: var(--ot-violet);
}
.ctx i {
  display: block;
  height: 8px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--ot-ink) 12%, transparent);
}
.ctx i:nth-child(3) { width: 80%; }
.ctx i:nth-child(4) { width: 60%; }
.toast {
  position: absolute;
  right: 14px;
  bottom: 14px;
  width: min(260px, calc(100% - 28px));
  pointer-events: none;
}
.toast-enter-active,
.toast-leave-active {
  transition: opacity .2s, transform .25s cubic-bezier(.2, .8, .2, 1);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(10px);
}
.timeline {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.track {
  position: relative;
  height: 8px;
  margin-top: 18px;
  border-radius: 999px;
  background: var(--ot-surface-2);
  border: 1px solid var(--ot-line);
}
.fill {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: var(--ot-grad);
  transform-origin: left;
  opacity: .9;
}
.fill.replayed {
  background: var(--ot-violet);
}
.mark {
  position: absolute;
  top: -5px;
  bottom: -5px;
  width: 2px;
  margin-left: -1px;
  background: var(--ot-sky);
}
.mark.hold {
  background: var(--ot-amber);
}
.mark em {
  position: absolute;
  bottom: calc(100% + 4px);
  left: 50%;
  transform: translateX(-50%);
  font-size: 11px;
  font-style: normal;
  white-space: nowrap;
  color: var(--ot-faint);
}
.readout {
  display: flex;
  justify-content: space-between;
  font: 12px var(--ot-mono);
  color: var(--ot-faint);
}
.pd-side {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  padding: 16px;
  border: 1px solid var(--ot-line);
  border-radius: 14px;
  background: var(--ot-bg-2);
}
.side-title {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: .06em;
  color: var(--ot-faint);
}
.rules {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: 13px;
  color: var(--ot-muted);
}
.rules b {
  display: inline-block;
  min-width: 40px;
  margin-right: 8px;
  font-weight: 650;
}
.c-replay { color: var(--ot-violet); }
.c-hit { color: var(--ot-mint); }
.c-sky { color: var(--ot-sky); }
@media (max-width: 860px) {
  .pd {
    grid-template-columns: 1fr;
  }
  .pad {
    height: 300px;
  }
}
</style>
