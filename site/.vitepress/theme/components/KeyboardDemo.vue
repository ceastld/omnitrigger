<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref } from 'vue'
import DemoLog from './DemoLog.vue'
import DemoToast from './DemoToast.vue'
import Keycap from './Keycap.vue'
import { useEventLog } from './eventLog'

// 网页模拟：按文档描述演示前缀暂存、命中消费与回放，不代表真实时序。
type Binding = { label: string; desc: string; run: () => void }

const lines = ref(['按住 CapsLock，用 I J K L 移动光标', 'O 是退格，W 弹出提示'])
const caret = reactive({ line: 1, col: lines.value[1].length })
const capsOn = ref(false)
const focused = ref(false)
const held = reactive(new Set<string>())
const consumed = new Set<string>()
let prefix: 'idle' | 'buffered' | 'consumed' | 'replayed' = 'idle'

const { entries, push } = useEventLog()
const toast = ref<{ id: number; combo: string[]; text: string } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | undefined

function showToast(combo: string[], text: string) {
  clearTimeout(toastTimer)
  toast.value = { id: Date.now(), combo, text }
  toastTimer = setTimeout(() => (toast.value = null), 2600)
}

function moveCaret(dx: number, dy: number) {
  if (dy) {
    caret.line = Math.min(lines.value.length - 1, Math.max(0, caret.line + dy))
    caret.col = Math.min(caret.col, lines.value[caret.line].length)
    return
  }
  let { line, col } = caret
  col += dx
  if (col < 0 && line > 0) {
    line -= 1
    col = lines.value[line].length
  } else if (col > lines.value[line].length && line < lines.value.length - 1) {
    line += 1
    col = 0
  }
  caret.line = line
  caret.col = Math.min(lines.value[line].length, Math.max(0, col))
}

function backspace() {
  const { line, col } = caret
  if (col > 0) {
    const text = lines.value[line]
    lines.value[line] = text.slice(0, col - 1) + text.slice(col)
    caret.col -= 1
  } else if (line > 0) {
    const prev = lines.value[line - 1]
    lines.value[line - 1] = prev + lines.value[line]
    lines.value.splice(line, 1)
    caret.line -= 1
    caret.col = prev.length
  }
}

function insert(ch: string) {
  const text = lines.value[caret.line]
  if (text.length >= 40) return
  lines.value[caret.line] = text.slice(0, caret.col) + ch + text.slice(caret.col)
  caret.col += 1
}

function newline() {
  if (lines.value.length >= 4) return
  const text = lines.value[caret.line]
  lines.value.splice(caret.line, 1, text.slice(0, caret.col), text.slice(caret.col))
  caret.line += 1
  caret.col = 0
}

const bindings: Record<string, Binding> = {
  KeyI: { label: 'I', desc: '发送 ↑', run: () => moveCaret(0, -1) },
  KeyJ: { label: 'J', desc: '发送 ←', run: () => moveCaret(-1, 0) },
  KeyK: { label: 'K', desc: '发送 ↓', run: () => moveCaret(0, 1) },
  KeyL: { label: 'L', desc: '发送 →', run: () => moveCaret(1, 0) },
  KeyO: { label: 'O', desc: '发送 Backspace', run: backspace },
  KeyW: { label: 'W', desc: '显示提示', run: () => showToast(['CapsLock', 'W'], 'CapsLock + W 已触发') },
  Space: { label: 'Space', desc: '打开触发面板', run: () => showToast(['CapsLock', 'Space'], '打开触发面板（右侧鼠标演示可试）') },
}

const names: Record<string, string> = {
  Space: 'Space', Backspace: 'Backspace', Enter: 'Enter', ArrowLeft: '←', ArrowRight: '→', ArrowUp: '↑', ArrowDown: '↓',
}
function keyName(code: string, key?: string) {
  if (names[code]) return names[code]
  if (code.startsWith('Key')) return code.slice(3)
  if (code.startsWith('Digit')) return code.slice(5)
  return key && key.length === 1 ? key : code
}

/** 透传给“前台窗口”。返回是否被模拟编辑器处理。 */
function passThrough(code: string, key: string | undefined, shift: boolean) {
  if (code === 'Backspace') backspace()
  else if (code === 'Enter') newline()
  else if (code === 'ArrowLeft') moveCaret(-1, 0)
  else if (code === 'ArrowRight') moveCaret(1, 0)
  else if (code === 'ArrowUp') moveCaret(0, -1)
  else if (code === 'ArrowDown') moveCaret(0, 1)
  else if (code.startsWith('Key')) {
    const letter = code.slice(3)
    insert(capsOn.value !== shift ? letter.toUpperCase() : letter.toLowerCase())
  } else if (code === 'Space') insert(' ')
  else if (key && key.length === 1) insert(key)
  else return false
  return true
}

function down(code: string, opts: { repeat?: boolean; key?: string; shift?: boolean } = {}) {
  if (code === 'CapsLock') {
    if (opts.repeat || prefix !== 'idle') return
    held.add(code)
    prefix = 'buffered'
    push('buffer', 'CapsLock 按下 → 暂存，等待下一键')
    return
  }
  if (opts.repeat) {
    if (consumed.has(code)) return
    passThrough(code, opts.key, !!opts.shift)
    return
  }
  held.add(code)
  const binding = bindings[code]
  if ((prefix === 'buffered' || prefix === 'consumed') && binding) {
    consumed.add(code)
    prefix = 'consumed'
    push('hit', `CapsLock + ${binding.label} → ${binding.desc}，消费整次按压`)
    binding.run()
    return
  }
  if (prefix === 'buffered') {
    prefix = 'replayed'
    capsOn.value = !capsOn.value
    push('replay', `${keyName(code, opts.key)} 不匹配 → 先回放 CapsLock`)
  }
  passThrough(code, opts.key, !!opts.shift)
  push('pass', `${keyName(code, opts.key)} → 透传给前台窗口`)
}

function up(code: string) {
  held.delete(code)
  if (code === 'CapsLock') {
    if (prefix === 'buffered') {
      capsOn.value = !capsOn.value
      push('replay', 'CapsLock 单独松开 → 回放按下与释放')
    } else if (prefix === 'consumed') {
      push('consume', 'CapsLock 释放 → 随整次按压一起消费')
    } else if (prefix === 'replayed') {
      push('pass', 'CapsLock 释放 → 透传')
    }
    prefix = 'idle'
    return
  }
  consumed.delete(code)
}

const handled = /^(Key[A-Z]|Digit\d|Space|Backspace|Enter|Arrow\w+|CapsLock|Minus|Equal|Comma|Period|Slash|Semicolon|Quote|Bracket\w+|Backslash)$/

function onKeydown(event: KeyboardEvent) {
  if (event.ctrlKey || event.altKey || event.metaKey) return
  if (!handled.test(event.code)) return
  event.preventDefault()
  down(event.code, { repeat: event.repeat, key: event.key, shift: event.shiftKey })
}
function onKeyup(event: KeyboardEvent) {
  if (!handled.test(event.code)) return
  event.preventDefault()
  up(event.code)
}
function onBlur() {
  focused.value = false
  for (const code of [...held]) up(code)
}

const root = ref<HTMLElement | null>(null)
const focusDemo = () => root.value?.focus({ preventScroll: true })

const virtualKeys = ['I', 'J', 'K', 'L', 'O', 'W']
function virtualDown(code: string) {
  focusDemo()
  down(code, { key: code === 'Space' ? ' ' : code.slice(3).toLowerCase() })
}
function toggleCaps() {
  focusDemo()
  if (held.has('CapsLock')) up('CapsLock')
  else down('CapsLock')
}

const rendered = computed(() =>
  lines.value.map((text, index) =>
    index === caret.line ? { before: text.slice(0, caret.col), after: text.slice(caret.col), caret: true } : { before: text, after: '', caret: false },
  ),
)

onBeforeUnmount(() => clearTimeout(toastTimer))
</script>

<template>
  <div
    ref="root"
    class="kd"
    :class="{ focused }"
    tabindex="0"
    role="application"
    aria-label="键盘前缀演示：获得焦点后用键盘输入"
    @keydown="onKeydown"
    @keyup="onKeyup"
    @focus="focused = true"
    @blur="onBlur"
  >
    <div class="kd-main">
      <div class="window">
        <div class="window-bar">
          <span class="dots"><i /><i /><i /></span>
          <span class="window-title">前台窗口（模拟）</span>
          <span class="caps" :class="{ on: capsOn }">大写锁定 {{ capsOn ? '开' : '关' }}</span>
        </div>
        <div class="editor" aria-live="polite">
          <div v-for="(line, index) in rendered" :key="index" class="line">
            <span>{{ line.before }}</span><span v-if="line.caret" class="cursor" /><span>{{ line.after }}</span>
          </div>
        </div>
        <transition name="toast">
          <DemoToast v-if="toast" :key="toast.id" class="toast" :text="toast.text" :combo="toast.combo" />
        </transition>
        <button v-if="!focused" type="button" class="focus-hint" tabindex="-1" @mousedown.prevent @click="focusDemo">
          <span>点击这里，然后按住 <b>CapsLock</b> 再按 <b>J</b> / <b>L</b><small>也可以直接点下方的屏幕按键</small></span>
        </button>
      </div>

      <div class="vkeys" aria-label="屏幕按键">
        <button
          type="button"
          tabindex="-1"
          class="vk caps-key"
          :class="{ held: held.has('CapsLock') }"
          @mousedown.prevent
          @click="toggleCaps"
        >
          <Keycap :pressed="held.has('CapsLock')">CapsLock</Keycap>
          <small>{{ held.has('CapsLock') ? '按住中 · 再点松开' : '点击按住' }}</small>
        </button>
        <span class="plus big">+</span>
        <button
          v-for="k in virtualKeys"
          :key="k"
          type="button"
          tabindex="-1"
          class="vk"
          @mousedown.prevent
          @pointerdown="virtualDown(`Key${k}`)"
          @pointerup="up(`Key${k}`)"
          @pointerleave="held.has(`Key${k}`) && up(`Key${k}`)"
        >
          <Keycap :pressed="held.has(`Key${k}`)">{{ k }}</Keycap>
        </button>
      </div>
    </div>

    <aside class="kd-side">
      <div class="side-title">引擎决策</div>
      <DemoLog :entries="entries" empty="按下按键后，这里显示暂存、命中、消费与回放。" />
      <div class="side-title">本演示的绑定</div>
      <ul class="bindings">
        <li><span><Keycap size="sm">CapsLock</Keycap> + <Keycap size="sm">I</Keycap><Keycap size="sm">J</Keycap><Keycap size="sm">K</Keycap><Keycap size="sm">L</Keycap></span><em>↑ ← ↓ →</em></li>
        <li><span><Keycap size="sm">CapsLock</Keycap> + <Keycap size="sm">O</Keycap></span><em>Backspace</em></li>
        <li><span><Keycap size="sm">CapsLock</Keycap> + <Keycap size="sm">W</Keycap></span><em>显示提示</em></li>
      </ul>
    </aside>
  </div>
</template>

<style scoped>
.kd {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(0, 1fr);
  gap: 18px;
  outline: none;
}
.kd-main {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}
.window {
  position: relative;
  min-height: 228px;
  border: 1px solid var(--ot-line);
  border-radius: 14px;
  background: var(--ot-surface);
  overflow: hidden;
  transition: border-color .2s, box-shadow .2s;
}
.kd.focused .window {
  border-color: var(--ot-line-strong);
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--ot-mint) 12%, transparent);
}
.window-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--ot-line);
  background: var(--ot-surface-2);
  font-size: 12px;
  color: var(--ot-faint);
}
.dots {
  display: inline-flex;
  gap: 6px;
}
.dots i {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--ot-line-strong);
  opacity: .5;
}
.window-title {
  flex: 1;
}
.caps {
  padding: 2px 8px;
  border: 1px solid var(--ot-line);
  border-radius: 999px;
  font-variant-numeric: tabular-nums;
}
.caps.on {
  border-color: var(--ot-amber);
  color: var(--ot-amber);
}
.editor {
  padding: 18px 20px 26px;
  font: 15px/1.9 var(--ot-mono);
  color: var(--ot-ink);
  word-break: break-all;
}
.line {
  min-height: 1.9em;
  white-space: pre-wrap;
}
.cursor {
  display: inline-block;
  width: 2px;
  height: 1.15em;
  margin: 0 -1px;
  vertical-align: -.2em;
  background: var(--ot-mint);
  box-shadow: 0 0 10px var(--ot-mint);
  animation: blink 1.05s steps(1) infinite;
}
.kd:not(.focused) .cursor {
  opacity: .35;
  animation: none;
}
@keyframes blink {
  50% { opacity: 0; }
}
.focus-hint {
  position: absolute;
  inset: 41px 0 0;
  display: grid;
  place-items: center;
  padding: 16px;
  border: 0;
  background: color-mix(in srgb, var(--ot-surface) 70%, transparent);
  backdrop-filter: blur(2px);
  color: var(--ot-muted);
  font-size: 14px;
  cursor: pointer;
}
.focus-hint b {
  color: var(--ot-mint);
  font-weight: 600;
}
.focus-hint small {
  display: block;
  margin-top: 6px;
  font-size: 12px;
  color: var(--ot-faint);
}
.toast {
  position: absolute;
  right: 14px;
  bottom: 14px;
  width: min(260px, calc(100% - 28px));
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
.plus {
  color: var(--ot-faint);
  font-size: 12px;
}
.plus.big {
  font-size: 18px;
  align-self: center;
  padding-bottom: 18px;
}
.vkeys {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: 8px;
}
.vk {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 0;
  border: 0;
  background: none;
  cursor: pointer;
  touch-action: manipulation;
  user-select: none;
}
.vk small {
  font-size: 11px;
  color: var(--ot-faint);
}
.vk.held small {
  color: var(--ot-mint);
}
.kd-side {
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
.bindings {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: 13px;
}
.bindings li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
}
.bindings span {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--ot-faint);
}
.bindings em {
  font-style: normal;
  color: var(--ot-muted);
}
@media (max-width: 860px) {
  .kd {
    grid-template-columns: 1fr;
  }
}
</style>
