// 构建时收集实际打包进浏览器脚本的第三方软件包许可，生成随站点发布的许可清单。
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright)(\.|-|$)/i

function packageRoot(id) {
  const path = id.replace(/\\/g, '/').split('?')[0]
  const index = path.lastIndexOf('/node_modules/')
  if (index < 0) return null
  const rest = path.slice(index + '/node_modules/'.length).split('/')
  const name = rest[0].startsWith('@') ? `${rest[0]}/${rest[1]}` : rest[0]
  return path.slice(0, index + '/node_modules/'.length) + name
}

function describe(root) {
  const manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
  const files = readdirSync(root)
    .filter((name) => LICENSE_FILE.test(name))
    .sort()
    .map((name) => ({ name, text: readFileSync(join(root, name), 'utf8').trim() }))
  const repository = typeof manifest.repository === 'string' ? manifest.repository : manifest.repository?.url
  return {
    name: manifest.name,
    version: manifest.version,
    license: manifest.license ?? manifest.licenses?.map((item) => item.type).join(' OR ') ?? 'package.json 未声明，见许可正文',
    source: (repository ?? manifest.homepage ?? '').replace(/^git\+/, '').replace(/\.git$/, ''),
    files,
  }
}

export function thirdPartyLicenses({ fileName = 'third-party-licenses.txt', title }) {
  return {
    name: 'omnitrigger:third-party-licenses',
    apply: 'build',
    generateBundle(options, bundle) {
      // VitePress 同时构建浏览器端与 SSR；只有浏览器端脚本随站点分发
      if (!options.dir || options.dir.replace(/\\/g, '/').includes('/.temp')) return
      const roots = new Set()
      for (const output of Object.values(bundle)) {
        if (output.type !== 'chunk') continue
        for (const id of Object.keys(output.modules)) {
          const root = packageRoot(id)
          if (root && existsSync(join(root, 'package.json'))) roots.add(root)
        }
      }
      const packages = [...roots].map(describe).sort((a, b) => a.name.localeCompare(b.name))
      const missing = packages.filter((item) => !item.files.length)
      const rule = '='.repeat(72)
      const lines = [
        title,
        '',
        '以下软件包被打包进本站的浏览器脚本，按各自的许可证分发；OmniTrigger 的 MIT 许可证不替代这些条款。',
        `共 ${packages.length} 个软件包。`,
        '',
      ]
      for (const item of packages) {
        lines.push(rule, `${item.name}@${item.version} — ${item.license}`)
        if (item.source) lines.push(item.source)
        if (!item.files.length) lines.push('', '（发布包未附带许可正文，许可声明见其 package.json。）')
        for (const file of item.files) lines.push('-'.repeat(72), `${file.name}`, '', file.text)
        lines.push('')
      }
      this.emitFile({ type: 'asset', fileName, source: lines.join('\n') })
      const note = missing.length ? `，其中 ${missing.length} 个未附带许可正文：${missing.map((item) => item.name).join('、')}` : ''
      this.info?.(`已收集 ${packages.length} 个第三方软件包许可${note}`)
    },
  }
}
