import { defineConfig } from 'vitepress'
import { REPO_BRANCH, REPO_URL, sidebar, syncDocs, watchDocs, workspaceVersion } from '../scripts/sync-docs.mjs'
import { thirdPartyLicenses } from '../scripts/third-party-licenses.mjs'

// 默认发布到 GitHub Pages 项目站点；改用自定义域名时设置 SITE_URL 与 SITE_BASE。
const siteUrl = (process.env.SITE_URL ?? 'https://ceastld.github.io/omnitrigger/').replace(/\/?$/, '/')
const base = process.env.SITE_BASE ?? new URL(siteUrl).pathname
const version = workspaceVersion()
const title = 'OmniTrigger'
const description = '统一录制、注册与管理键盘和鼠标组合的 Rust 触发引擎。组合键、长按、快速拖拽与屏幕边角，绑定提示、快捷键、程序、浮动面板或动作轮盘。'

syncDocs()
if (process.argv.includes('dev') && !(globalThis as { __otDocsWatch?: boolean }).__otDocsWatch) {
  ;(globalThis as { __otDocsWatch?: boolean }).__otDocsWatch = true
  watchDocs()
}

export default defineConfig({
  lang: 'zh-CN',
  title,
  titleTemplate: ':title · OmniTrigger',
  description,
  base,
  cleanUrls: true,
  appearance: 'dark',
  lastUpdated: false,
  sitemap: { hostname: siteUrl },

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}favicon.svg` }],
    ['meta', { name: 'theme-color', content: '#07090c' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: title }],
    ['meta', { property: 'og:image', content: `${siteUrl}og.png` }],
    ['meta', { property: 'og:image:width', content: '1200' }],
    ['meta', { property: 'og:image:height', content: '630' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
  ],

  transformPageData(page) {
    const pageTitle = page.frontmatter.title ?? page.title
    const pageDescription = page.frontmatter.description ?? description
    const path = page.relativePath.replace(/(^|\/)index\.md$/, '$1').replace(/\.md$/, '')
    page.frontmatter.head ??= []
    page.frontmatter.head.push(
      ['meta', { property: 'og:title', content: pageTitle ? `${pageTitle} · ${title}` : title }],
      ['meta', { property: 'og:description', content: pageDescription }],
      ['meta', { property: 'og:url', content: `${siteUrl}${path}` }],
    )
  },

  vite: {
    define: {
      __OT_VERSION__: JSON.stringify(version),
      __OT_REPO__: JSON.stringify(REPO_URL),
    },
    plugins: [thirdPartyLicenses({ title: 'OmniTrigger 主页与文档站随附的第三方软件' })],
    build: {
      // mermaid 按需加载，只在含流程图的文档页下载
      chunkSizeWarningLimit: 1600,
    },
  },

  markdown: {
    theme: { light: 'github-light', dark: 'github-dark-dimmed' },
    config(md) {
      const fence = md.renderer.rules.fence!
      md.renderer.rules.fence = (tokens, index, options, env, self) => {
        const token = tokens[index]
        if (token.info.trim() === 'mermaid') {
          return `<Mermaid code="${encodeURIComponent(token.content)}" />`
        }
        return fence(tokens, index, options, env, self)
      }
    },
  },

  themeConfig: {
    logo: { src: '/logo.svg', alt: '' },
    siteTitle: 'OmniTrigger',

    nav: [
      { text: '快速开始', link: '/docs/getting-started', activeMatch: '^/docs/getting-started' },
      { text: '文档', link: '/docs/', activeMatch: '^/docs/(?!getting-started|releases)' },
      { text: '发布说明', link: `/docs/releases/v${version}`, activeMatch: '^/docs/releases/' },
      { text: `下载 v${version}`, link: `${REPO_URL}/releases` },
    ],

    sidebar: { '/docs/': sidebar() },

    socialLinks: [{ icon: 'github', link: REPO_URL, ariaLabel: 'GitHub 仓库' }],

    editLink: {
      pattern: (page) =>
        `https://github.com/ceastld/omnitrigger/edit/main/${page.frontmatter.source ?? ''}`,
      text: '在 GitHub 上编辑此页',
    },

    search: {
      provider: 'local',
      options: {
        miniSearch: {
          options: {
            // 中文按词切分，函数会序列化到浏览器端，须保持自包含
            tokenize: (text: string) => {
              const Seg = (Intl as { Segmenter?: typeof Intl.Segmenter }).Segmenter
              if (!Seg) return text.split(/[\n\r\p{Z}\p{P}]+/u)
              const words: string[] = []
              for (const part of new Seg('zh-CN', { granularity: 'word' }).segment(text)) {
                if (part.isWordLike) words.push(part.segment)
              }
              return words
            },
          },
          searchOptions: { prefix: true, fuzzy: 0.15, combineWith: 'AND' },
        },
        translations: {
          button: { buttonText: '搜索文档', buttonAriaLabel: '搜索文档' },
          modal: {
            displayDetails: '显示详细列表',
            resetButtonTitle: '清除查询',
            backButtonTitle: '关闭搜索',
            noResultsText: '没有找到相关结果',
            footer: { selectText: '选择', navigateText: '切换', closeText: '关闭' },
          },
        },
      },
    },

    outline: { level: [2, 3], label: '本页目录' },
    docFooter: { prev: '上一页', next: '下一页' },
    darkModeSwitchLabel: '外观',
    lightModeSwitchTitle: '切换到浅色模式',
    darkModeSwitchTitle: '切换到深色模式',
    sidebarMenuLabel: '文档目录',
    returnToTopLabel: '回到顶部',
    skipToContentLabel: '跳到正文',
    langMenuLabel: '语言',
    externalLinkIcon: true,
    notFound: {
      title: '页面不存在',
      quote: '这个组合还没有注册。',
      linkLabel: '返回首页',
      linkText: '返回首页',
    },

    footer: {
      message: `基于 <a href="${REPO_URL}/blob/${REPO_BRANCH}/LICENSE">MIT 许可证</a> 发布 · <a href="${base}third-party-licenses.txt">本站第三方许可</a> · <a href="${REPO_URL}/blob/${REPO_BRANCH}/THIRD_PARTY_NOTICES.md">程序第三方通知</a>`,
      copyright: `OmniTrigger v${version} · Windows 预览版`,
    },
  },
})
