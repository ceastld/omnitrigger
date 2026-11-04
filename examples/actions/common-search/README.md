# 文本搜索脚本

`web-search` 是可直接编辑的标准同步 JS。`source` 必须明确指定为 `selection` 或 `text`。前者向原目标发送复制命令并读取新文本，复制结果留在剪贴板；后者只使用调用参数 `text`，不读取桌面或剪贴板。

`urlTemplate` 默认是 `https://www.google.com/search?q=%s`。可改成带路径的 HTTP(S) 地址，路径或查询中必须恰好包含一个 `%s`。脚本先检查模板和参数，再复制选区；搜索词用 `encodeURIComponent` 编码，不作为原始 URL 拼接。

`maxCodePoints` 默认 128，范围 1–2048。截断按 Unicode 码点计数，不拆开代理对。文本保留原始空格和换行；纯空白、NUL 和无效 Unicode 会报错。空选区不会回退到旧剪贴板。

这个示例只做文本网页搜索。它不会检查文本是否为本地文件，也不执行文件、自动选择浏览器或复用已有标签页。需要搜索之外的分支时，应在自己的脚本中明确实现。选区复制的重试和超时由现有宿主 API 决定，脚本不额外覆盖。

`submitted` 表示系统接纳打开 URL 的请求，不能证明网页加载成功。返回值只包含来源、搜索词码点数与是否截断，不返回搜索词或完整 URL。

纯脚本检查不会读取真实选区或打开浏览器：

```powershell
node --test examples/actions/common-search/recipes.test.mjs
```
