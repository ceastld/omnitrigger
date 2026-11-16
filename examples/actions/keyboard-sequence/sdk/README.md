# 固定 SDK 类型声明

以下文件逐字节复制自 OmniTrigger 公开提交 `de1aeadca5b9a7848a1ed0480c8e27464663ee49` 的 `crates/omnitrigger-actions/runtime/`，采用项目 MIT 许可证。

- [api.d.ts](https://github.com/ceastld/omnitrigger/blob/de1aeadca5b9a7848a1ed0480c8e27464663ee49/crates/omnitrigger-actions/runtime/api.d.ts)，Git blob `b271c743145f25ca44528100f5d6bdb2ee936def`，SHA-256 `ebd8e81dce8483c993601c688d140284e37538070a682d098874e74f13f27191`。
- [script-types.d.ts](https://github.com/ceastld/omnitrigger/blob/de1aeadca5b9a7848a1ed0480c8e27464663ee49/crates/omnitrigger-actions/runtime/script-types.d.ts)，Git blob `628ff976a9b5dacd2103b88f645fe20d0285ccf6`，SHA-256 `d8f4d1b5b575f2eb3f624723cce47c3c098970272f12b8f757922993fe3ad94a`。
- [clipboard-history-api.d.ts](https://github.com/ceastld/omnitrigger/blob/de1aeadca5b9a7848a1ed0480c8e27464663ee49/crates/omnitrigger-actions/runtime/clipboard-history-api.d.ts)，Git blob `e62ea381ceb8ff823d3815a30e265980bb1572ed`，SHA-256 `07a0f1c009ec384da046193a91825169d94854dd2afd68a91b1e53adb6c6cdc7`。

`../sdk.d.ts` 是本包的本地类型入口。声明不提供运行时代码，不新增 ctx API。更新 SDK 时应成组替换所需声明并运行本包严格检查。
