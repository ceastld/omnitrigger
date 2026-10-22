# 第三方依赖与资源

[MIT 许可证](LICENSE) 适用于本项目自身的代码、文档和资源。第三方依赖保留各自的版权与许可要求；项目的 MIT 声明不替代这些要求。

## Rust 依赖

精确版本以提交的 `Cargo.lock` 为准。主要直接依赖如下，包含构建工具：

| 依赖 | 用途 | 许可证声明 |
| --- | --- | --- |
| GPUI | Windows 桌面界面 | Apache-2.0 |
| windows | Windows API 绑定 | MIT OR Apache-2.0 |
| serde / serde_json | 本地配置序列化 | MIT OR Apache-2.0 |
| unicode-segmentation | 文本编辑的字素边界 | MIT OR Apache-2.0 |
| raw-window-handle | 原生窗口句柄 | MIT OR Apache-2.0 OR Zlib |
| embed-resource | 构建时嵌入 Windows 图标资源 | MIT |

`OR` 表示可按上游条款选择其中一种许可。完整依赖图还包含传递依赖，不能仅凭此表打包可执行文件。

可查看当前 Windows 目标的依赖与包元数据：

```powershell
cargo tree --locked --target x86_64-pc-windows-msvc
cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
```

初次准备开源时，核对了锁定依赖的许可证声明与包内的许可、版权文件。GPUI 0.2.2 使用 Apache-2.0，其许可文件包含 Zed Industries, Inc. 的版权声明；部分传递依赖另有 COPYRIGHT 文件。全平台依赖中还含 MPL-2.0 的 `cbindgen`（其他平台的构建依赖），扩展平台时需重新检查。

该核对不能替代针对实际发行包的完整审查。分发二进制前，应按实际目标、功能与锁定版本收集许可证全文、版权及适用通知，随包提供；依赖变更后重新生成。Apache-2.0 的发行要求见[官方许可第 4 节](https://www.apache.org/licenses/LICENSE-2.0)。

## 图标与字体

程序图标的 SVG 源文件与 ICO 位于 `crates/omnitrigger-app/assets/`，界面小图标以内联 SVG 保存在 `src/assets.rs`。这些文件随项目源码提供。引入外部素材时，应同时记录来源并保留其许可声明。

界面请求使用系统安装的 Microsoft YaHei UI、Cascadia Mono 等字体，仓库没有附带字体文件。系统字体不因本项目使用 MIT 而获得再分发许可。
