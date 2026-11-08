# 文件与文件夹预览动作

已有 Node 入口使用显式 `roots`，未提供时读取当前调用捕获的 Explorer 上下文。目录菜单按 24 项分页，最多五轮，打开前重新检查目标类型；目录结果不完整或超限时直接报错。

在此目录安装固定开发依赖并检查：

```powershell
npm ci --ignore-scripts
npm run check
```

开发依赖固定 TypeScript 5.9.3 与 Node 22.20.5 声明，来源与完整性由锁文件记录。检查使用本工程依赖，验证严格类型及 Node ESM 默认导出，不运行入口、不枚举用户文件或打开目标。动作实际运行需要 Node.js 22 或更高版本和应用提供的本次调用上下文。

在 VS Code 中打开本目录即可发现 `jsconfig.json`。SDK 位于 `sdk/`，只为此 Node 工程加载；不加入浏览器全局。仓库内也可使用既有统一检查器：

```powershell
node tools/script-sdk-check/check-project-types.mjs "examples/actions/source-file-preview"
```

SDK 可用 `tools/script-project.mjs refresh --runtime node --project <本目录>` 显式刷新。刷新保留作者入口、清单、配置与说明，不安装依赖或运行作者代码。当前包提供 Node 入口，不能直接作为同步 Script 运行。

需要选择菜单时，通过应用内运行入口或正常绑定执行；独立 CLI 不创建选择窗口。
