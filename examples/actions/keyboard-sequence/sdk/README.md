# 固定 SDK 类型声明

以下文件逐字节复制自 OmniTrigger 公开提交 `ec93f4c43f544ae18eef7dbc3f62d886a35ea67e` 的 `crates/omnitrigger-actions/runtime/`，采用项目 MIT 许可证。

- [api.d.ts](https://github.com/ceastld/omnitrigger/blob/ec93f4c43f544ae18eef7dbc3f62d886a35ea67e/crates/omnitrigger-actions/runtime/api.d.ts)，Git blob `1bdb682311b00e4765f3e86877b6b4a0275e9401`，SHA-256 `5101d124402fd17c1b6302fa78c58c1eaabfa1ac4751128ed99fabf49d04aba2`。
- [script-types.d.ts](https://github.com/ceastld/omnitrigger/blob/ec93f4c43f544ae18eef7dbc3f62d886a35ea67e/crates/omnitrigger-actions/runtime/script-types.d.ts)，Git blob `c86d1678c5bf61bc4d0aa4ab4404a2231b006824`，SHA-256 `fa36e4c11c7ec7d3e79b5f7b4015733a0cd03f417c3a29f59d484448675eca02`。
- [clipboard-history-api.d.ts](https://github.com/ceastld/omnitrigger/blob/ec93f4c43f544ae18eef7dbc3f62d886a35ea67e/crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts)，Git blob `e62ea381ceb8ff823d3815a30e265980bb1572ed`，SHA-256 `07a0f1c009ec384da046193a91825169d94854dd2afd68a91b1e53adb6c6cdc7`。

`../sdk.d.ts` 是本包的本地类型入口。声明不提供运行时代码，不新增 ctx API。更新 SDK 时应成组替换所需声明并运行本包严格检查。
