# 固定 SDK 类型声明

以下文件逐字节复制自 OmniTrigger 公开提交 `e51a45b8e312e4edc71ee4003039b0875842bb3e` 的 `crates/omnitrigger-actions/runtime/`，采用项目 MIT 许可证。

- [api.d.ts](https://github.com/ceastld/omnitrigger/blob/e51a45b8e312e4edc71ee4003039b0875842bb3e/crates/omnitrigger-actions/runtime/api.d.ts)，Git blob `ae3c550a34d8e841ff945d0630cf0a83ae34bedf`，SHA-256 `12020f626e467a1025b8eb9b08245952f0425f97a983a8f66c48a1864e6d7adc`。
- [script-types.d.ts](https://github.com/ceastld/omnitrigger/blob/e51a45b8e312e4edc71ee4003039b0875842bb3e/crates/omnitrigger-actions/runtime/script-types.d.ts)，Git blob `20c19f9867f617e86bc0fcf875bb15fc1340a38d`，SHA-256 `261666bf8be38669cbbe16f27fa2bf44f3a20f2ded90cef2be888de85a939f01`。
- [clipboard-history-api.d.ts](https://github.com/ceastld/omnitrigger/blob/e51a45b8e312e4edc71ee4003039b0875842bb3e/crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts)，Git blob `2e1ebb83e2efb93e4eec7e29a63e6e20b69295ae`，SHA-256 `c40fd66fd6318909c1bab62775d097dcfb5a8a7a84a411883a4852bdbd04cf2e`。

`../sdk.d.ts` 是本包的本地类型入口。声明不提供运行时代码，不新增 ctx API。更新 SDK 时应成组替换所需声明并运行本包严格检查。
