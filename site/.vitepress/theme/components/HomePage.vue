<script setup lang="ts">
import { withBase } from 'vitepress'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import KeyboardDemo from './KeyboardDemo.vue'
import Keycap from './Keycap.vue'
import PointerDemo from './PointerDemo.vue'

const version = __OT_VERSION__
const repo = __OT_REPO__
const releases = `${repo}/releases`

// 首屏轮播的组合均为当前已实现的触发方式与动作。
const combos = [
  { keys: ['CapsLock', 'J'], via: '按下', action: '发送 ←', kind: '发送快捷键' },
  { keys: ['右键'], via: '长按 300ms', action: '浮动触发面板', kind: '面板' },
  { keys: ['右键'], via: '快拖 18px', action: '动作轮盘', kind: '轮盘' },
  { keys: ['左边缘', '滚轮 ↑'], via: '按下', action: '音量 +5', kind: '系统调节' },
  { keys: ['左 Ctrl', '左 Alt', 'A'], via: '按下', action: '截图', kind: '插件动作' },
  { keys: ['侧键 1', '滚轮 ↓'], via: '按下', action: '运行脚本', kind: '程序与脚本' },
]
const comboIndex = ref(0)
const pressedCount = ref(0)
let ticker: ReturnType<typeof setInterval> | undefined
const combo = computed(() => combos[comboIndex.value])

onMounted(() => {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
    pressedCount.value = 99
    return
  }
  let step = 0
  ticker = setInterval(() => {
    step += 1
    const keys = combo.value.keys.length
    if (step <= keys) pressedCount.value = step
    else if (step >= keys + 6) {
      step = 0
      pressedCount.value = 0
      comboIndex.value = (comboIndex.value + 1) % combos.length
    }
  }, 380)
})
onBeforeUnmount(() => clearInterval(ticker))

const demo = ref<'keyboard' | 'pointer'>('keyboard')

const anatomy = [
  {
    step: '01',
    title: '触发方式',
    lead: '同一个键，可以有不同的触发时机。',
    items: [
      ['按下即触发', '按住最多 8 个前缀，再按目标键或滚轮方向'],
      ['长按', '50–10000ms，同一次按压只触发一次'],
      ['快速拖拽', '4–500px、20–2000ms 的严格起步时窗'],
    ],
  },
  {
    step: '02',
    title: '位置条件',
    lead: '让同一个组合只在屏幕的某处生效。',
    items: [
      ['四个角落 · 四条边', '使用显示器完整范围，排除多屏连接处'],
      ['自定义矩形', '物理屏幕坐标，支持负坐标'],
      ['可与任何方式组合', '位置可证明互斥时允许分别绑定'],
    ],
  },
  {
    step: '03',
    title: '动作',
    lead: '触发后做什么，独立保存、独立测试。',
    items: [
      ['提示 · 快捷键 · 本地步骤', '文字、剪贴板、变量、打开与等待'],
      ['窗口 · 音量 · 亮度', '最大化 / 最小化，DDC/CI 硬件亮度'],
      ['程序脚本 · 面板 · 轮盘', '可选安装截图插件动作'],
    ],
  },
]

const tour = [
  {
    key: 'triggers',
    tab: '触发器',
    title: '录制、检查、注册，一次完成',
    desc: '按住前缀再按目标键或滚轮，全部松开即完成录制。录完立即按注册规则检查，指出冲突的触发器，不覆盖原有绑定。',
    points: ['组合以键帽显示，区分左右修饰键', '按下即触发、长按、快速拖拽三种方式', '本次运行的命中次数与最近时间'],
    img: '/screenshots/triggers.webp',
    w: 1325,
    h: 1013,
    doc: '/docs/features',
  },
  {
    key: 'panel',
    tab: '触发面板',
    title: '不抢焦点的浮动面板',
    desc: '以鼠标位置为中心显示三列动作卡片，原窗口保持输入焦点。数字键直达，方向键选择后回车运行，Esc 或点击外部收起。',
    points: ['右键长按 300ms 或 CapsLock + Space 呼出', '点击筛选框才取得前台，收起时交还', '停用、执行中或未就绪的动作不可运行'],
    img: '/screenshots/panel.webp',
    w: 708,
    h: 295,
    float: true,
    doc: '/docs/trigger-panel',
  },
  {
    key: 'wheel',
    tab: '轨道轮盘',
    title: '快拖呼出，松开执行',
    desc: '右键快速拖拽打开轮盘，按固定起点的方向选槽，松开原鼠标键执行；回到起点 9px 内取消。可按前台应用切换整组槽位。',
    points: ['最多 16 个轮盘，三圈稀疏方位，每组最多 48 槽', '薄荷、紫罗兰、琥珀三种配色', '普通呼出也支持点击、数字键与方向键'],
    img: '/screenshots/wheel.webp',
    w: 650,
    h: 650,
    float: true,
    doc: '/docs/radial-wheels',
  },
  {
    key: 'presets',
    tab: '触发预设',
    title: '23 个可编辑模板',
    desc: '屏幕边角、右键快拖、右键 / 中键 / 侧键长按、键盘长按和常用组合按五组展示。选择预设只填写草稿，保存时经过同一套冲突检查。',
    points: ['右键快拖可与同组合的静止长按共存', '四条边、四个角落，排除多屏连接处', '屏幕边缘滚轮可调节音量与亮度'],
    img: '/screenshots/presets.webp',
    w: 1325,
    h: 1013,
    doc: '/docs/trigger-presets',
  },
  {
    key: 'library',
    tab: '动作库',
    title: '动作与触发分开配置',
    desc: '本地步骤组合快捷键、文字、剪贴板、变量、打开与等待；还有窗口操作、音量与亮度、程序脚本和可选的截图插件。',
    points: ['动作在 Hook 之外的后台执行器运行', '前台窗口切换会取消待发送的按键', '插件安装校验 SHA-256，不自动注册快捷键'],
    img: '/screenshots/library.webp',
    w: 1325,
    h: 1013,
    doc: '/docs/local-actions',
  },
  {
    key: 'diagnostics',
    tab: '输入诊断',
    title: '漏触发时有据可查',
    desc: '每秒更新键盘 / 鼠标回调数、外部模拟输入和当前按住的键，以及长按判定与面板提交耗时。偶发失效可一键重建监听。',
    points: ['统计只在内存中，不记录原始按键历史', '区分候选被取消与显示较慢', '重建监听保留配置与暂停状态'],
    img: '/screenshots/diagnostics.webp',
    w: 1325,
    h: 1013,
    doc: '/docs/troubleshooting',
  },
]
const tourIndex = ref(0)
const shot = computed(() => tour[tourIndex.value])

const flow = [
  { title: 'Windows 低级 Hook', sub: '采集键鼠事件，识别自身注入', zone: 'sync' },
  { title: '标准化事件', sub: 'InputEvent + 显式时间与上下文', zone: 'sync' },
  { title: 'omnitrigger-core', sub: '纯 Rust 确定性状态机', zone: 'sync', core: true },
  { title: '决策', sub: '透传 · 消费 · 触发通知 · 补发计划', zone: 'sync' },
  { title: '应用层', sub: '后台执行器、界面、磁盘与网络', zone: 'async' },
]

const configSample = `{
  "version": 7,
  "bindings": [
    {
      "id": 1,
      "held": [],
      "activation": {
        "kind": "hold",
        "value": {
          "button": { "kind": "right" },
          "duration_ms": 300
        }
      },
      "message": "打开触发面板",
      "enabled": true,
      "action": { "kind": "show_panel" }
    }
  ]
}`

const limits = [
  ['补发会改变时序', '前缀暂存后回放、右键短按回放都会改变原始时序，不能承诺完全无损。'],
  ['受系统约束', 'Windows 安全桌面和系统保留组合仍受系统限制；向权限更高的应用补发可能失败，此时暂停监听并提示。'],
  ['与其他 Hook 并存', '其他软件在上游拦截的输入，OmniTrigger 收不到；诊断页帮助判断，但不能仅凭漏触发认定来源。'],
  ['尚未实现', '双击 / 连击、按设备区分、系统托盘、插件市场与自动更新。'],
  ['预览阶段', '仅 Windows x64，程序未签名；API 与配置格式可能调整。'],
]

const privacy = [
  '不提供遥测、云同步或输入上传。',
  '全局监听只用于本机匹配与录制，触发记录和输入诊断只保留在进程内。',
  '配置保存为本地 JSON，默认位于 %LOCALAPPDATA%\\OmniTrigger。',
  '只有在你安装截图插件时，才访问 GitHub 下载程序与运行依赖。',
  '项目以 MIT 许可证开源，全部源码可以审阅。',
]
</script>

<template>
  <div class="ot-home">
    <!-- 首屏 -->
    <section class="hero">
      <div class="aura" aria-hidden="true"><i /><i /><i /></div>
      <div class="wrap hero-grid">
        <div class="hero-copy">
          <a class="pill" :href="withBase(`/docs/releases/v${version}`)">
            <span class="pill-dot" />v{{ version }} · Windows 预览版<span class="pill-link">发布说明 →</span>
          </a>
          <h1>录下一个操作，<br /><span class="grad">它就是你的触发器</span></h1>
          <p class="lead">
            OmniTrigger 统一录制、注册与管理键盘和鼠标组合。按住前缀再按一键、长按、快速拖拽、在屏幕边缘滚动——录下来，绑定提示、快捷键、本地步骤、程序脚本、浮动面板或动作轮盘。
          </p>

          <div class="ticker" aria-live="off">
            <transition name="swap" mode="out-in">
              <div :key="comboIndex" class="ticker-row">
                <span class="ticker-keys">
                  <template v-for="(k, i) in combo.keys" :key="k">
                    <span v-if="i" class="plus">+</span>
                    <Keycap :pressed="pressedCount > i">{{ k }}</Keycap>
                  </template>
                  <span v-if="combo.via !== '按下'" class="via" :class="{ on: pressedCount > combo.keys.length - 1 }">{{ combo.via }}</span>
                </span>
                <span class="arrow" :class="{ on: pressedCount >= combo.keys.length }">→</span>
                <span class="result" :class="{ on: pressedCount >= combo.keys.length }">
                  <b>{{ combo.action }}</b><small>{{ combo.kind }}</small>
                </span>
              </div>
            </transition>
          </div>

          <div class="actions">
            <a class="btn primary" :href="releases" target="_blank" rel="noopener">
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12m0 0-5-5m5 5 5-5M4 19h16" /></svg>
              下载 Windows 预览版
            </a>
            <a class="btn" :href="withBase('/docs/getting-started')">快速开始</a>
            <a class="btn ghost" :href="repo" target="_blank" rel="noopener">
              <svg viewBox="0 0 24 24" aria-hidden="true" class="gh"><path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.64 0 0 .84-.27 2.75 1.02a9.6 9.6 0 0 1 5 0c1.91-1.3 2.75-1.02 2.75-1.02.55 1.37.2 2.39.1 2.64.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2Z" /></svg>
              源码
            </a>
          </div>
          <p class="meta">Windows x64 · Rust + GPUI 桌面程序 · 无遥测 · MIT 开源</p>
        </div>

        <div class="hero-visual">
          <figure class="frame main-shot">
            <img :src="withBase('/screenshots/triggers.webp')" width="1325" height="1013" alt="OmniTrigger 触发器管理界面：触发器列表、组合键帽与触发器详情" fetchpriority="high" />
          </figure>
          <figure class="float-shot panel-shot">
            <img :src="withBase('/screenshots/panel.webp')" width="708" height="295" alt="浮动触发面板：筛选框与三列动作卡片" loading="lazy" />
          </figure>
          <figure class="float-shot wheel-shot">
            <img :src="withBase('/screenshots/wheel.webp')" width="650" height="650" alt="轨道轮盘：复制、粘贴、全选、撤销四个槽位" loading="lazy" />
          </figure>
        </div>
      </div>
    </section>

    <!-- 交互演示 -->
    <section id="try" class="section">
      <div class="wrap">
        <header class="section-head">
          <p class="kicker">在浏览器里试一试</p>
          <h2>前缀不会白白吃掉你的按键</h2>
          <p>
            已注册的前缀按下时先暂存：组合命中就消费整次按压，不匹配或单独松开就回放原输入。同一个右键也能区分短按、快速拖拽和静止长按。下面是按文档语义编写的网页模拟。
          </p>
        </header>
        <div class="demo-card">
          <div class="tabs" role="tablist">
            <button type="button" role="tab" :aria-selected="demo === 'keyboard'" :class="{ on: demo === 'keyboard' }" @click="demo = 'keyboard'">
              <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2.5" y="6" width="19" height="12" rx="2.5" /><path d="M6.5 10h1m3 0h1m3 0h1m3 0h.5M7 14h10" /></svg>键盘前缀
            </button>
            <button type="button" role="tab" :aria-selected="demo === 'pointer'" :class="{ on: demo === 'pointer' }" @click="demo = 'pointer'">
              <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="3" width="12" height="18" rx="6" /><path d="M12 3v6M6 9h12" /></svg>右键手势
            </button>
          </div>
          <KeyboardDemo v-if="demo === 'keyboard'" />
          <PointerDemo v-else />
          <p class="demo-note">
            网页模拟只演示决策顺序，按 CSS 像素计算；浏览器无法拦截系统 CapsLock 状态。桌面程序在 Windows Hook 中判断，补发会改变输入时序，不能承诺完全无损。
          </p>
        </div>
      </div>
    </section>

    <!-- 触发器构成 -->
    <section class="section alt">
      <div class="wrap">
        <header class="section-head">
          <p class="kicker">一个触发器的构成</p>
          <h2>触发方式 × 位置条件 × 动作</h2>
          <p>三者分别配置、自由组合。录制、注册、冲突检查与运行时匹配共用同一份 <code>TriggerSpec</code>：录到什么，就按什么匹配。</p>
        </header>
        <div class="anatomy">
          <article v-for="col in anatomy" :key="col.step" class="anatomy-card">
            <div class="step">{{ col.step }}</div>
            <h3>{{ col.title }}</h3>
            <p class="anatomy-lead">{{ col.lead }}</p>
            <ul>
              <li v-for="[name, detail] in col.items" :key="name">
                <b>{{ name }}</b><span>{{ detail }}</span>
              </li>
            </ul>
          </article>
        </div>
        <div class="conflict">
          <div class="conflict-icon" aria-hidden="true">!</div>
          <p>
            <b>冲突会说清楚，不会静默覆盖。</b>
            重复组合、目标键被其他组合用作前缀、同组合的普通按下与长按争用等情况，都会指出具体的触发器。一个长按与一个快拖可以共存，只要快拖时窗严格短于长按阈值。
          </p>
        </div>
      </div>
    </section>

    <!-- 界面导览 -->
    <section class="section">
      <div class="wrap">
        <header class="section-head">
          <p class="kicker">界面导览</p>
          <h2>一个窗口，管理全部触发器</h2>
          <p>以下均为当前版本的真实界面截图，使用首次运行的示例配置。</p>
        </header>
        <div class="tour">
          <div class="tour-tabs" role="tablist">
            <button
              v-for="(item, i) in tour"
              :key="item.key"
              type="button"
              role="tab"
              :aria-selected="i === tourIndex"
              :class="{ on: i === tourIndex }"
              @click="tourIndex = i"
            >
              <span class="tour-num">0{{ i + 1 }}</span>{{ item.tab }}
            </button>
          </div>
          <div class="tour-body">
            <transition name="fade" mode="out-in">
              <div :key="shot.key" class="tour-slide">
                <figure class="tour-shot" :class="{ float: shot.float }">
                  <img :src="withBase(shot.img)" :width="shot.w" :height="shot.h" :alt="`${shot.tab}界面截图`" loading="lazy" />
                </figure>
                <div class="tour-text">
                  <h3>{{ shot.title }}</h3>
                  <p>{{ shot.desc }}</p>
                  <ul>
                    <li v-for="p in shot.points" :key="p">{{ p }}</li>
                  </ul>
                  <a class="more" :href="withBase(shot.doc)">阅读文档 →</a>
                </div>
              </div>
            </transition>
          </div>
        </div>
      </div>
    </section>

    <!-- 引擎设计 -->
    <section class="section alt">
      <div class="wrap">
        <header class="section-head">
          <p class="kicker">引擎设计</p>
          <h2>确定性核心，Hook 里只做有界匹配</h2>
          <p>核心是一个纯 Rust 状态机，接收标准化事件、时间与上下文，产生决策；动作、磁盘、网络都交给 Hook 之外的执行器。</p>
        </header>
        <div class="flow" role="list">
          <div v-for="(node, i) in flow" :key="node.title" class="flow-node" :class="[node.zone, { core: node.core }]" role="listitem">
            <span class="flow-idx">{{ i + 1 }}</span>
            <b>{{ node.title }}</b>
            <small>{{ node.sub }}</small>
          </div>
        </div>
        <div class="flow-legend">
          <span class="sync">Hook 同步路径：只做有界状态更新与匹配</span>
          <span class="async">Hook 之外：执行动作、补发与读写配置</span>
        </div>
        <div class="engine-grid">
          <div class="engine-cards">
            <article>
              <h3>相同输入，相同决策</h3>
              <p>核心不读取系统时钟，不依赖 UI 或 Windows API；鼠标位置与屏幕矩形由平台显式提供。前缀回退、释放、重复与注入边界都用事件序列验证。</p>
            </article>
            <article>
              <h3>不会自己触发自己</h3>
              <p>自身补发带专用标记，其他软件注入的输入也不触发、不参与录制，避免动作连锁自触发。</p>
            </article>
            <article>
              <h3>配置可读、可检查</h3>
              <p>绑定保存为版本化 JSON，旧版本首次保存前自动备份。<code>--check-config</code> 只读验证格式、参数、冲突与轮盘引用。</p>
            </article>
          </div>
          <figure class="code-card">
            <figcaption><span>examples/right-hold-panel.json</span><em>右键长按 300ms → 触发面板</em></figcaption>
            <pre><code>{{ configSample }}</code></pre>
          </figure>
        </div>
        <p class="engine-foot">
          三个 crate 各司其职：<code>omnitrigger-core</code> 无外部依赖，可在 Linux 上独立测试；<code>omnitrigger-windows</code> 隔离原生 API；<code>omnitrigger-app</code> 负责 GPUI 界面与动作分派。
          <a :href="withBase('/docs/architecture')">工程架构 →</a>
        </p>
      </div>
    </section>

    <!-- 边界与隐私 -->
    <section class="section">
      <div class="wrap">
        <header class="section-head">
          <p class="kicker">如实说明</p>
          <h2>能做到的写清楚，做不到的也写清楚</h2>
        </header>
        <div class="honest">
          <div class="honest-col">
            <h3>当前边界</h3>
            <dl>
              <template v-for="[title, detail] in limits" :key="title">
                <dt>{{ title }}</dt>
                <dd>{{ detail }}</dd>
              </template>
            </dl>
            <a class="more" :href="withBase('/docs/input-semantics')">输入语义与边界 →</a>
          </div>
          <div class="honest-col privacy">
            <h3>输入只在本机匹配</h3>
            <ul>
              <li v-for="item in privacy" :key="item">{{ item }}</li>
            </ul>
            <a class="more" :href="withBase('/docs/configuration')">配置文件与隐私 →</a>
          </div>
        </div>
      </div>
    </section>

    <!-- 下载 -->
    <section class="section cta">
      <div class="wrap">
        <div class="cta-card">
          <div class="cta-copy">
            <img class="cta-logo" :src="withBase('/logo.svg')" alt="" width="64" height="64" />
            <h2>把常用操作，交给一个组合</h2>
            <p>解压即用，无需安装驱动。首次启动加载示例绑定，先按住 <Keycap size="sm">CapsLock</Keycap> 再按 <Keycap size="sm">W</Keycap> 试试。</p>
            <div class="actions">
              <a class="btn primary" :href="releases" target="_blank" rel="noopener">下载 v{{ version }} 预览版</a>
              <a class="btn" :href="withBase('/docs/getting-started')">快速开始</a>
              <a class="btn ghost" :href="withBase('/docs/building')">从源码构建</a>
            </div>
          </div>
          <ul class="cta-reqs">
            <li><b>系统</b>Windows x64，建议 Windows 11</li>
            <li><b>运行库</b>Microsoft Visual C++ v14 Redistributable x64</li>
            <li><b>发布包</b>ZIP，附 SHA256SUMS.txt 与依赖许可</li>
            <li><b>状态</b>早期预览，未签名，不提供自动更新</li>
          </ul>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.ot-home {
  background: var(--ot-bg);
  color: var(--ot-ink);
  overflow-x: clip;
}
.wrap {
  width: min(1200px, calc(100% - 48px));
  margin: 0 auto;
}
code {
  padding: 2px 6px;
  border-radius: 6px;
  background: var(--ot-surface-2);
  font: .88em var(--ot-mono);
  color: var(--ot-mint);
}

/* 首屏 */
.hero {
  position: relative;
  padding: 72px 0 96px;
  isolation: isolate;
}
.aura {
  position: absolute;
  inset: -20% 0 0;
  z-index: -1;
  overflow: hidden;
  pointer-events: none;
}
.aura i {
  position: absolute;
  width: 56vmax;
  height: 56vmax;
  border-radius: 50%;
  filter: blur(100px);
  opacity: .22;
  animation: drift 30s ease-in-out infinite alternate;
}
.aura i:nth-child(1) { background: #12b07a; left: -18vmax; top: -18vmax; }
.aura i:nth-child(2) { background: #0f86b8; right: -22vmax; top: 4vmax; animation-delay: -10s; }
.aura i:nth-child(3) { background: #4c3bb0; left: 30vw; top: 40vmax; opacity: .12; animation-delay: -18s; }
:root:not(.dark) .aura i { opacity: .12; }
@keyframes drift {
  to { transform: translate3d(4vmax, 3vmax, 0) scale(1.08); }
}
.hero-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.08fr);
  align-items: center;
  gap: 56px;
}
.pill {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px 6px 10px;
  border: 1px solid var(--ot-line-strong);
  border-radius: 999px;
  background: var(--ot-grad-soft);
  font-size: 13px;
  color: var(--ot-ink);
  text-decoration: none;
  transition: border-color .2s;
}
.pill:hover {
  border-color: var(--ot-mint);
}
.pill-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--ot-mint);
  box-shadow: 0 0 10px var(--ot-mint);
}
.pill-link {
  color: var(--ot-mint);
}
h1 {
  margin: 22px 0 18px;
  font-size: clamp(38px, 5.2vw, 62px);
  font-weight: 750;
  line-height: 1.12;
  letter-spacing: -.02em;
}
.grad {
  background: var(--ot-grad);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
.lead {
  max-width: 560px;
  margin: 0;
  font-size: 17px;
  line-height: 1.75;
  color: var(--ot-muted);
}
.ticker {
  min-height: 64px;
  margin: 28px 0 30px;
}
.ticker-row {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border: 1px solid var(--ot-line);
  border-radius: 16px;
  background: color-mix(in srgb, var(--ot-surface) 80%, transparent);
}
.ticker-keys {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.plus {
  color: var(--ot-faint);
}
.via {
  padding: 4px 9px;
  border: 1px dashed var(--ot-line-strong);
  border-radius: 8px;
  font-size: 12px;
  color: var(--ot-faint);
  transition: color .2s, border-color .2s;
}
.via.on {
  border-color: var(--ot-mint);
  color: var(--ot-mint);
}
.arrow {
  color: var(--ot-faint);
  transition: color .2s, transform .2s;
}
.arrow.on {
  color: var(--ot-mint);
  transform: translateX(2px);
}
.result {
  display: inline-flex;
  flex-direction: column;
  line-height: 1.25;
  opacity: .35;
  transition: opacity .25s;
}
.result.on {
  opacity: 1;
}
.result b {
  font-size: 15px;
}
.result small {
  font-size: 11px;
  color: var(--ot-faint);
}
.swap-enter-active,
.swap-leave-active {
  transition: opacity .22s, transform .22s;
}
.swap-enter-from {
  opacity: 0;
  transform: translateY(8px);
}
.swap-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
.btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 46px;
  padding: 0 20px;
  border: 1px solid var(--ot-line-strong);
  border-radius: 12px;
  background: var(--ot-surface);
  color: var(--ot-ink);
  font-size: 15px;
  font-weight: 600;
  text-decoration: none;
  transition: transform .15s, border-color .2s, box-shadow .2s, background .2s;
}
.btn:hover {
  transform: translateY(-1px);
  border-color: var(--ot-mint);
}
.btn svg {
  width: 18px;
  height: 18px;
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.btn svg.gh {
  fill: currentColor;
  stroke: none;
}
.btn.primary {
  border-color: transparent;
  background: var(--ot-grad);
  color: #04110c;
  box-shadow: 0 10px 30px -10px color-mix(in srgb, var(--ot-mint) 70%, transparent);
}
.btn.primary:hover {
  box-shadow: 0 14px 36px -10px color-mix(in srgb, var(--ot-mint) 90%, transparent);
}
.btn.ghost {
  background: transparent;
  border-color: var(--ot-line);
}
.meta {
  margin: 18px 0 0;
  font-size: 13px;
  color: var(--ot-faint);
}
.hero-visual {
  position: relative;
  padding: 0 0 60px 0;
}
.frame {
  margin: 0;
  border: 1px solid var(--ot-line-strong);
  border-radius: 14px;
  background: #0a0c10;
  box-shadow: var(--ot-shadow), 0 0 0 1px rgba(0, 0, 0, .4);
  overflow: hidden;
}
.frame img,
.float-shot img {
  display: block;
  width: 100%;
  height: auto;
}
.main-shot {
  transform: perspective(1800px) rotateY(-9deg) rotateX(3deg);
  transform-origin: left center;
}
.float-shot {
  position: absolute;
  margin: 0;
  filter: drop-shadow(0 24px 40px rgba(0, 0, 0, .55));
}
.panel-shot {
  left: -7%;
  bottom: 0;
  width: 52%;
  animation: bob 7s ease-in-out infinite;
}
.wheel-shot {
  right: -3%;
  top: -12%;
  width: 32%;
  animation: bob 8s ease-in-out -3s infinite;
}
@keyframes bob {
  50% { transform: translateY(-8px); }
}

/* 通用分区 */
.section {
  padding: 104px 0;
}
.section.alt {
  background: var(--ot-bg-2);
  border-block: 1px solid var(--ot-line);
}
.section-head {
  max-width: 760px;
  margin-bottom: 44px;
}
.kicker {
  margin: 0 0 10px;
  font-size: 13px;
  font-weight: 650;
  letter-spacing: .14em;
  color: var(--ot-mint);
}
h2 {
  margin: 0 0 16px;
  padding: 0;
  border: 0;
  font-size: clamp(28px, 3.4vw, 40px);
  font-weight: 720;
  line-height: 1.2;
  letter-spacing: -.015em;
}
.section-head p:not(.kicker) {
  margin: 0;
  font-size: 16px;
  line-height: 1.8;
  color: var(--ot-muted);
}
h3 {
  margin: 0;
  font-size: 19px;
  font-weight: 680;
}
.more {
  display: inline-block;
  margin-top: 18px;
  font-size: 14px;
  font-weight: 600;
  color: var(--ot-mint);
  text-decoration: none;
}
.more:hover {
  text-decoration: underline;
}

/* 演示 */
.demo-card {
  padding: 18px;
  border: 1px solid var(--ot-line);
  border-radius: 20px;
  background: color-mix(in srgb, var(--ot-surface) 55%, var(--ot-bg));
  box-shadow: var(--ot-shadow);
}
.tabs {
  display: inline-flex;
  gap: 4px;
  margin-bottom: 18px;
  padding: 4px;
  border: 1px solid var(--ot-line);
  border-radius: 12px;
  background: var(--ot-bg-2);
}
.tabs button {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-radius: 9px;
  font-size: 14px;
  font-weight: 600;
  color: var(--ot-muted);
  transition: background .2s, color .2s;
}
.tabs button svg {
  width: 17px;
  height: 17px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.6;
  stroke-linecap: round;
}
.tabs button.on {
  background: var(--ot-surface-2);
  color: var(--ot-ink);
  box-shadow: inset 0 0 0 1px var(--ot-line-strong);
}
.demo-note {
  margin: 16px 2px 0;
  font-size: 12.5px;
  line-height: 1.7;
  color: var(--ot-faint);
}

/* 构成 */
.anatomy {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 18px;
}
.anatomy-card {
  position: relative;
  padding: 26px 24px 24px;
  border: 1px solid var(--ot-line);
  border-radius: 18px;
  background: var(--ot-surface);
}
.anatomy-card:not(:last-child)::after {
  content: '×';
  position: absolute;
  top: 50%;
  right: -16px;
  z-index: 1;
  display: grid;
  place-items: center;
  width: 14px;
  color: var(--ot-faint);
  font-size: 18px;
  transform: translateY(-50%);
}
.step {
  margin-bottom: 14px;
  font: 600 12px var(--ot-mono);
  color: var(--ot-mint);
}
.anatomy-lead {
  margin: 6px 0 18px;
  font-size: 14px;
  color: var(--ot-muted);
}
.anatomy-card ul {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.anatomy-card li {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 12px 14px;
  border: 1px solid var(--ot-line);
  border-radius: 12px;
  background: var(--ot-surface-2);
}
.anatomy-card li b {
  font-size: 14px;
}
.anatomy-card li span {
  font-size: 13px;
  color: var(--ot-faint);
}
.conflict {
  display: flex;
  gap: 16px;
  align-items: flex-start;
  margin-top: 22px;
  padding: 18px 20px;
  border: 1px solid color-mix(in srgb, var(--ot-amber) 35%, transparent);
  border-radius: 16px;
  background: color-mix(in srgb, var(--ot-amber) 7%, transparent);
}
.conflict-icon {
  flex: none;
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--ot-amber) 22%, transparent);
  color: var(--ot-amber);
  font-weight: 800;
}
.conflict p {
  margin: 0;
  font-size: 14.5px;
  line-height: 1.75;
  color: var(--ot-muted);
}
.conflict b {
  color: var(--ot-ink);
}

/* 导览 */
.tour {
  display: grid;
  grid-template-columns: 200px minmax(0, 1fr);
  gap: 28px;
}
.tour-tabs {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.tour-tabs button {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-radius: 12px;
  font-size: 15px;
  font-weight: 600;
  color: var(--ot-muted);
  text-align: left;
  transition: background .2s, color .2s;
}
.tour-tabs button:hover {
  color: var(--ot-ink);
}
.tour-tabs button.on {
  background: var(--ot-surface);
  color: var(--ot-ink);
  box-shadow: inset 2px 0 0 var(--ot-mint), inset 0 0 0 1px var(--ot-line);
}
.tour-num {
  font: 600 11px var(--ot-mono);
  color: var(--ot-faint);
}
.tour-tabs button.on .tour-num {
  color: var(--ot-mint);
}
.tour-slide {
  display: grid;
  grid-template-columns: minmax(0, 1.55fr) minmax(0, 1fr);
  gap: 32px;
  align-items: center;
}
.tour-shot {
  display: grid;
  place-items: center;
  min-height: 300px;
  margin: 0;
  border: 1px solid var(--ot-line-strong);
  border-radius: 14px;
  background: #0a0c10;
  box-shadow: var(--ot-shadow);
  overflow: hidden;
}
.tour-shot img {
  display: block;
  width: 100%;
  height: auto;
}
.tour-shot.float {
  padding: 36px;
  background:
    radial-gradient(circle at 50% 40%, rgba(82, 238, 176, .14), transparent 60%),
    radial-gradient(circle at 1px 1px, rgba(255, 255, 255, .05) 1px, transparent 0) 0 0 / 20px 20px,
    #0a0d12;
}
.tour-shot.float img {
  width: auto;
  max-width: 100%;
  max-height: 420px;
  filter: drop-shadow(0 20px 40px rgba(0, 0, 0, .6));
}
.tour-text p {
  margin: 12px 0 16px;
  font-size: 15px;
  line-height: 1.8;
  color: var(--ot-muted);
}
.tour-text ul {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.tour-text li {
  position: relative;
  padding-left: 18px;
  font-size: 14px;
  color: var(--ot-muted);
}
.tour-text li::before {
  content: '';
  position: absolute;
  left: 2px;
  top: .62em;
  width: 6px;
  height: 6px;
  border-radius: 2px;
  background: var(--ot-mint);
}
.fade-enter-active,
.fade-leave-active {
  transition: opacity .18s, transform .18s;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
  transform: translateY(6px);
}

/* 引擎 */
.flow {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 14px;
}
.flow-node {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 16px;
  border: 1px solid var(--ot-line);
  border-radius: 14px;
  background: var(--ot-surface);
}
.flow-node:not(:last-child)::after {
  content: '';
  position: absolute;
  top: 50%;
  right: -12px;
  width: 10px;
  height: 2px;
  background: var(--ot-line-strong);
}
.flow-node.core {
  border-color: var(--ot-line-strong);
  background: linear-gradient(160deg, color-mix(in srgb, var(--ot-mint) 12%, var(--ot-surface)), var(--ot-surface));
}
.flow-node.async {
  border-style: dashed;
  border-color: color-mix(in srgb, var(--ot-sky) 45%, transparent);
}
.flow-idx {
  font: 600 11px var(--ot-mono);
  color: var(--ot-faint);
}
.flow-node b {
  font-size: 15px;
}
.flow-node small {
  font-size: 12.5px;
  line-height: 1.55;
  color: var(--ot-faint);
}
.flow-legend {
  display: grid;
  grid-template-columns: 4fr 1fr;
  gap: 14px;
  margin: 12px 0 36px;
  font-size: 12px;
}
.flow-legend span {
  padding-top: 8px;
  border-top: 2px solid var(--ot-mint);
  color: var(--ot-muted);
}
.flow-legend .async {
  border-top: 2px dashed var(--ot-sky);
}
.engine-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 22px;
}
.engine-cards {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.engine-cards article {
  padding: 20px 22px;
  border: 1px solid var(--ot-line);
  border-radius: 16px;
  background: var(--ot-surface);
}
.engine-cards h3 {
  font-size: 16px;
}
.engine-cards p {
  margin: 8px 0 0;
  font-size: 14px;
  line-height: 1.75;
  color: var(--ot-muted);
}
.code-card {
  display: flex;
  flex-direction: column;
  margin: 0;
  border: 1px solid var(--ot-line);
  border-radius: 16px;
  background: #0b0f14;
  overflow: hidden;
}
.code-card figcaption {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding: 12px 16px;
  border-bottom: 1px solid rgba(150, 255, 214, .08);
  font: 12px var(--ot-mono);
  color: #7b8a85;
}
.code-card figcaption em {
  font-style: normal;
  color: #5ee8b0;
}
.code-card pre {
  flex: 1;
  margin: 0;
  padding: 16px 18px;
  overflow: auto;
  font: 13px/1.65 var(--ot-mono);
  color: #c9d6d1;
}
.code-card code {
  padding: 0;
  background: none;
  color: inherit;
  font: inherit;
}
.engine-foot {
  margin: 26px 0 0;
  font-size: 14px;
  line-height: 1.8;
  color: var(--ot-muted);
}
.engine-foot a {
  color: var(--ot-mint);
  font-weight: 600;
  text-decoration: none;
  white-space: nowrap;
}

/* 边界与隐私 */
.honest {
  display: grid;
  grid-template-columns: 1.2fr 1fr;
  gap: 22px;
}
.honest-col {
  padding: 26px 28px;
  border: 1px solid var(--ot-line);
  border-radius: 18px;
  background: var(--ot-surface);
}
.honest-col h3 {
  margin-bottom: 18px;
}
.honest dl {
  display: grid;
  grid-template-columns: 120px minmax(0, 1fr);
  gap: 12px 18px;
  margin: 0;
}
.honest dt {
  font-size: 14px;
  font-weight: 650;
  color: var(--ot-amber);
}
.honest dd {
  margin: 0;
  font-size: 14px;
  line-height: 1.7;
  color: var(--ot-muted);
}
.privacy {
  background: linear-gradient(160deg, color-mix(in srgb, var(--ot-mint) 8%, var(--ot-surface)), var(--ot-surface));
  border-color: var(--ot-line-strong);
}
.privacy ul {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.privacy li {
  position: relative;
  padding-left: 26px;
  font-size: 14.5px;
  line-height: 1.7;
  color: var(--ot-muted);
}
.privacy li::before {
  content: '✓';
  position: absolute;
  left: 0;
  top: 1px;
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--ot-mint) 18%, transparent);
  color: var(--ot-mint);
  font-size: 11px;
  font-weight: 800;
}

/* 下载 */
.cta {
  padding-top: 40px;
}
.cta-card {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  gap: 36px;
  align-items: center;
  padding: 44px;
  border: 1px solid var(--ot-line-strong);
  border-radius: 24px;
  background:
    radial-gradient(circle at 10% 0%, color-mix(in srgb, var(--ot-mint) 16%, transparent), transparent 55%),
    radial-gradient(circle at 100% 100%, color-mix(in srgb, var(--ot-sky) 14%, transparent), transparent 55%),
    var(--ot-surface);
  box-shadow: var(--ot-shadow);
}
.cta-logo {
  display: block;
  width: 64px;
  height: 64px;
  margin-bottom: 18px;
}
.cta-copy p {
  margin: 0 0 24px;
  font-size: 15.5px;
  line-height: 1.9;
  color: var(--ot-muted);
}
.cta-reqs {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.cta-reqs li {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 12px 16px;
  border: 1px solid var(--ot-line);
  border-radius: 12px;
  background: color-mix(in srgb, var(--ot-bg) 50%, transparent);
  font-size: 14px;
  color: var(--ot-muted);
}
.cta-reqs b {
  font-size: 12px;
  font-weight: 650;
  letter-spacing: .06em;
  color: var(--ot-faint);
}

/* 响应式 */
@media (max-width: 1080px) {
  .hero-grid {
    grid-template-columns: 1fr;
    gap: 40px;
  }
  .hero-visual {
    max-width: 760px;
    margin: 0 auto;
    padding-left: 4%;
  }
  .flow {
    grid-template-columns: 1fr;
    gap: 10px;
  }
  .flow-node:not(:last-child)::after {
    top: auto;
    bottom: -9px;
    right: auto;
    left: 28px;
    width: 2px;
    height: 8px;
  }
  .flow-legend {
    grid-template-columns: 1fr;
  }
  .tour-slide {
    grid-template-columns: 1fr;
  }
}
@media (max-width: 860px) {
  .wrap {
    width: calc(100% - 32px);
  }
  .hero {
    padding: 40px 0 64px;
  }
  .section {
    padding: 72px 0;
  }
  .anatomy,
  .engine-grid,
  .honest,
  .cta-card {
    grid-template-columns: 1fr;
  }
  .anatomy-card:not(:last-child)::after {
    top: auto;
    bottom: -17px;
    right: 50%;
    transform: translateX(50%);
  }
  .tour {
    grid-template-columns: 1fr;
    gap: 16px;
  }
  .tour-tabs {
    flex-direction: row;
    overflow-x: auto;
    padding-bottom: 4px;
    scrollbar-width: none;
  }
  .tour-tabs button {
    flex: none;
    padding: 9px 12px;
    font-size: 14px;
  }
  .honest dl {
    grid-template-columns: 1fr;
    gap: 4px;
  }
  .honest dd {
    margin-bottom: 10px;
  }
  .cta-card {
    padding: 28px 22px;
  }
  .main-shot {
    transform: none;
  }
  .demo-card {
    padding: 12px;
  }
}
@media (max-width: 520px) {
  .ticker-row {
    gap: 8px;
    padding: 10px 12px;
  }
  .btn {
    flex: 1 1 auto;
    justify-content: center;
  }
}
</style>
