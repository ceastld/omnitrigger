# 显式 Edit 选区行清理（实验）

独立示例动作包，提供删除空行与删除整行空白两种操作。Node 异步入口、Script 具名同步 main 入口。以当前 SDK 的显式 Win32 Edit 替换 API 提交，不自动切换适配器。

调用 input 必须提供 adapter：native-edit 对应原生 Unicode 标准 Edit，winforms-edit 对应受支持的 WinForms Edit 类。operation 为 remove-empty-lines（默认）或 remove-white-space-lines。四份 call-*.json 使用显式 winforms-edit；请按实际目标选择，不适配时返回错误，不自动猜测。

两种操作复用 selection-line-cleanup 的行转换语义：CRLF/CR/LF 拆分，固定 CRLF 连接；空白判定使用既有 .NET 字符集合，不 trim 保留行。空选区与未变化时跳过；非空选区清理后为空时，两次读取一致后提交一次空字符串，删除当前选区。读取上限8192 UTF16/reject；转换若因 CRLF 增长超过8192，在第二次读取前拒绝，不截断。

动作最多读取两次、提交一次：两次只读选区文本不一致即拒绝；一致后提交当前 Edit 选区。这些调用不构成原子操作；同样文本可能来自不同范围，读取后仍可改变选区，因此无法保证替换读取时的原范围。新 API 在空选区时可按原生语义插入；本动作的两次读取不能消除此竞态。调用期间需要保持同一受支持的原目标。

回执 replacement_message_processed/effectState unknown 仅表示写消息处理返回，未验证全部正文接受。超时、写后取消、目标变化或未知结果不重试、不回滚；不回退到键盘、剪贴板或其他控件。上限不构成硬期限或跨应用兼容性保证。

支持范围仅为明确选择适配器的标准 Unicode Win32/WinForms Edit。其他应用、富文本控件和自绘编辑器需要单独适配；本包不保证跨应用兼容性。未配置个人绑定。

开发验证：在本目录运行 node --test recipes.test.mjs；node tools/script-sdk-check/node_modules/typescript/lib/tsc.js -p <本目录>/jsconfig.json（编译器路径以项目根为基准）。Node 需要 await；同步 Script 直接返回，取消交由实际宿主处理。
