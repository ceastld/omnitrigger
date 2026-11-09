# 界面设计预览

用 React 组件在浏览器里预览和迭代界面设计，目前覆盖触发器页中栏：列表、键盘和鼠标三种视图，以及搜索、筛选与分组。

预览不自己解释触发器。行摘要、筛选维度、说明文字、键位布局、每条触发器点亮的键位与鼠标部位、深浅两套配色和图标，都由应用用同一套代码导出（`--export-trigger-preview`）；组件只负责绘制和交互状态，尺寸取自应用的 GPUI 代码，对应关系写在各组件文件开头。

## 使用

需要 Node.js 20.19 或更高版本。

```powershell
cd design
npm install --ignore-scripts
npm run dev        # http://127.0.0.1:5178
```

地址参数可直接打开某个状态，便于截图对照：`?view=keyboard`、`?view=mouse`、`?theme=light`、`?width=800`。

## 数据

- 默认数据 `data/ui-browse.json` 由仓库夹具 `tests/fixtures/profiles/ui-browse` 导出。修改导出格式、配色、图标或夹具后运行 `npm run export` 重新生成；单元测试会核对该文件与应用当前导出一致。
- 预览自己的数据：`npm run export -- --profile ui-browse data/local/ui-browse.json`（`data/local/` 不提交），然后在页面上“打开导出文件”。参数原样转交给应用，`--profile` 与 `--config` 都可用。
- 导出时动作按可用处理，不连接动作服务；未就绪原因只在应用内显示。

## 改设计的流程

1. 在这里改组件，用夹具数据和自己的数据看效果。
2. 确定后把同样的尺寸和配色改进 `crates/omnitrigger-app/src` 对应的渲染函数。
3. 涉及数据含义的改动（新的筛选维度、键位、部位）先改 Rust 导出，再改组件。
