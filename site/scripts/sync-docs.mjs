// 将仓库中的 Markdown 文档同步到 site/docs，供 VitePress 生成文档站。
// 仓库文档是唯一来源：这里只改写链接并补充 frontmatter，不改正文。
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, watch, writeFileSync } from 'node:fs'
import { dirname, posix, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const REPO_URL = 'https://github.com/ceastld/omnitrigger'
export const REPO_BRANCH = 'main'

const siteDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repoDir = resolve(siteDir, '..')
const outDir = resolve(siteDir, 'docs')

// 文档站收录的页面与侧栏分组。docs/ 下新增文档需在此登记，否则同步失败。
export const groups = [
  {
    text: '开始',
    items: [
      ['docs/README.md', 'index', '文档首页'],
      ['docs/GETTING_STARTED.md', 'getting-started', '快速开始'],
      ['docs/FEATURES.md', 'features', '当前功能与交互'],
      ['docs/PRODUCT.md', 'product', '产品理念'],
    ],
  },
  {
    text: '触发与输入',
    items: [
      ['docs/TRIGGER_PRESETS.md', 'trigger-presets', '触发预设、长按与快拖'],
      ['docs/POINTER_REGIONS.md', 'pointer-regions', '鼠标区域与系统调节'],
      ['docs/INPUT_SEMANTICS.md', 'input-semantics', '输入语义与边界'],
    ],
  },
  {
    text: '面板与轮盘',
    items: [
      ['docs/TRIGGER_PANEL.md', 'trigger-panel', '浮动触发面板'],
      ['docs/RADIAL_WHEELS.md', 'radial-wheels', '自定义轮盘'],
    ],
  },
  {
    text: '动作',
    items: [
      ['docs/KEYBOARD_ACTIONS.md', 'keyboard-actions', '发送按键与快捷键'],
      ['docs/LOCAL_ACTIONS.md', 'local-actions', '本地步骤动作'],
      ['docs/COMMAND_ACTIONS.md', 'command-actions', '程序与脚本'],
      ['docs/ACTION_PACKAGES.md', 'action-packages', '动作包与 JavaScript 入口'],
      ['docs/PLUGIN_SYSTEM.md', 'plugin-system', '插件动作与快速绑定'],
    ],
  },
  {
    text: '配置与排查',
    items: [
      ['docs/CONFIGURATION.md', 'configuration', '配置文件与隐私'],
      ['docs/TROUBLESHOOTING.md', 'troubleshooting', '并用其他工具与诊断'],
    ],
  },
  {
    text: '开发与发布',
    items: [
      ['docs/BUILDING.md', 'building', '从源码构建'],
      ['docs/ARCHITECTURE.md', 'architecture', '工程架构'],
      ['docs/PROFILES.md', 'profiles', '配置档与数据目录设计'],
      ['docs/ACCOUNT_WORKSPACE.md', 'account-workspace', '账号工作区与同步设计'],
      ['docs/BROWSER_EXTENSION.md', 'browser-extension', 'Chrome API 桥接扩展设计'],
      ['docs/EXECUTION_ARCHITECTURE.md', 'execution-architecture', '动作执行架构'],
      ['CONTRIBUTING.md', 'contributing', '贡献指南'],
      ['SECURITY.md', 'security', '安全说明'],
      ['docs/RELEASING.md', 'releasing', '开源与发布检查'],
      ['THIRD_PARTY_NOTICES.md', 'third-party-notices', '第三方通知'],
    ],
  },
]

function releasePages() {
  const dir = resolve(repoDir, 'docs/releases')
  return readdirSync(dir)
    .filter((name) => /^v\d+\.\d+\.\d+.*\.md$/.test(name))
    .map((name) => {
      const version = name.slice(1, -3)
      return [`docs/releases/${name}`, `releases/v${version}`, `v${version}`, version]
    })
    .sort((a, b) => compareVersions(b[3], a[3]))
    .map(([src, slug, text]) => [src, slug, text])
}

function compareVersions(a, b) {
  const pa = a.split(/[.-]/).map((part) => Number.parseInt(part, 10) || 0)
  const pb = b.split(/[.-]/).map((part) => Number.parseInt(part, 10) || 0)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0)
  }
  return 0
}

export function allGroups() {
  return [...groups, { text: '发布说明', items: releasePages() }]
}

export function sidebar() {
  return allGroups().map((group) => ({
    text: group.text,
    collapsed: false,
    items: group.items.map(([, slug, text]) => ({ text, link: `/docs/${slug === 'index' ? '' : slug}` })),
  }))
}

/** 工作区版本，取自根 Cargo.toml。 */
export function workspaceVersion() {
  const cargo = readFileSync(resolve(repoDir, 'Cargo.toml'), 'utf8')
  const section = cargo.split(/^\[workspace\.package\]\s*$/m)[1] ?? ''
  const match = section.match(/^version\s*=\s*"([^"]+)"/m)
  if (!match) throw new Error('无法从 Cargo.toml 读取 workspace.package.version')
  return match[1]
}

function pageMap() {
  const map = new Map()
  for (const group of allGroups()) {
    for (const [src, slug] of group.items) map.set(src, `${slug}.md`)
  }
  return map
}

function checkCoverage(map) {
  const missing = readdirSync(resolve(repoDir, 'docs'))
    .filter((name) => name.endsWith('.md'))
    .map((name) => `docs/${name}`)
    .filter((src) => !map.has(src))
  if (missing.length) {
    throw new Error(`以下文档未登记到文档站，请在 site/scripts/sync-docs.mjs 中加入：\n  ${missing.join('\n  ')}`)
  }
}

function githubUrl(repoPath) {
  const full = resolve(repoDir, repoPath)
  if (!existsSync(full)) return null
  const kind = statSync(full).isDirectory() ? 'tree' : 'blob'
  return `${REPO_URL}/${kind}/${REPO_BRANCH}/${repoPath.replace(/\/$/, '')}`
}

function rewriteLink(target, src, out, map, problems) {
  if (/^([a-z][a-z0-9+.-]*:|#|\/)/i.test(target)) return target
  const hashIndex = target.indexOf('#')
  const path = hashIndex >= 0 ? target.slice(0, hashIndex) : target
  const hash = hashIndex >= 0 ? target.slice(hashIndex) : ''
  const resolved = posix.normalize(posix.join(posix.dirname(src), decodeURI(path)))
  const page = map.get(resolved)
  if (page) {
    let rel = posix.relative(posix.dirname(out), page)
    if (!rel.startsWith('.')) rel = `./${rel}`
    return `${rel}${hash}`
  }
  const url = githubUrl(resolved)
  if (!url) {
    problems.push(`${src}: 链接目标不存在 ${target}`)
    return target
  }
  return `${url}${hash}`
}

function transform(text, src, out, map, problems) {
  let fence = null
  return text
    .split('\n')
    .map((line) => {
      const marker = line.match(/^\s*(`{3,}|~{3,})/)
      if (marker) {
        if (!fence) fence = marker[1]
        else if (marker[1][0] === fence[0] && marker[1].length >= fence.length) fence = null
        return line
      }
      if (fence) return line
      // 链接文字可含行内代码；独立的行内代码原样保留
      return line.replace(/(`[^`]*`)|(!?\[(?:[^\]`]|`[^`]*`)*\]\()([^)\s]+)(\))/g, (whole, code, open, target, close) =>
        code ? whole : `${open}${rewriteLink(target, src, out, map, problems)}${close}`,
      )
    })
    .join('\n')
}

// 程序图标同样以仓库资源为唯一来源。
const icons = [
  ['crates/omnitrigger-app/assets/omnitrigger.svg', 'public/logo.svg'],
  ['crates/omnitrigger-app/assets/omnitrigger-small.svg', 'public/favicon.svg'],
]

export function syncDocs() {
  for (const [src, dst] of icons) copyFileSync(resolve(repoDir, src), resolve(siteDir, dst))
  const map = pageMap()
  checkCoverage(map)
  rmSync(outDir, { recursive: true, force: true })
  const problems = []
  for (const [src, out] of map) {
    const body = readFileSync(resolve(repoDir, src), 'utf8').replace(/\r\n/g, '\n')
    const frontmatter = ['---', `source: ${src}`, '---', ''].join('\n')
    const file = resolve(outDir, out)
    mkdirSync(dirname(file), { recursive: true })
    writeFileSync(file, frontmatter + transform(body, src, out, map, problems))
  }
  if (problems.length) throw new Error(`文档链接检查失败：\n  ${problems.join('\n  ')}`)
  return map.size
}

/** 开发模式下监听仓库文档，修改后重新同步。 */
export function watchDocs(log = console.log) {
  const sources = [...pageMap().keys()].map((src) => resolve(repoDir, src))
  const dirs = new Set(sources.map((file) => dirname(file)))
  let timer = null
  for (const dir of dirs) {
    watch(dir, (_, name) => {
      if (!name || !name.endsWith('.md')) return
      clearTimeout(timer)
      timer = setTimeout(() => {
        try {
          syncDocs()
          log(`[docs] 已重新同步 ${name}`)
        } catch (error) {
          log(`[docs] ${error.message}`)
        }
      }, 120)
    })
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const count = syncDocs()
  console.log(`已同步 ${count} 篇文档到 ${posix.join('site', 'docs')}`)
}
