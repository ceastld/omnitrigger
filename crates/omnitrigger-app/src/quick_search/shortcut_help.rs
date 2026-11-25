use super::{csv_tools::write_rows, examples::search_key};

const SHORTCUTS: &[[&str; 3]] = &[
    ["输入", "Ctrl+←", "按词向左移动光标"],
    ["输入", "Ctrl+→", "按词向右移动光标"],
    ["输入", "Ctrl+Shift+←", "按词向左调整选区"],
    ["输入", "Ctrl+Shift+→", "按词向右调整选区"],
    ["输入", "Ctrl+Backspace", "按词向左删除"],
    ["输入", "Ctrl+Delete", "按词向右删除"],
    ["多行输入", "Alt+Home", "跳到当前行首"],
    ["多行输入", "Alt+End", "跳到当前行尾"],
    ["多行输入", "Alt+Shift+Home", "选中到当前行首"],
    ["多行输入", "Alt+Shift+End", "选中到当前行尾"],
    ["多行输入", "Ctrl+L", "选中当前行文字"],
    ["输入", "Ctrl+Z", "撤销编辑"],
    ["输入", "Ctrl+Y", "重做编辑"],
    ["输入", "Ctrl+Shift+Z", "重做编辑"],
    ["输入", "Shift+Enter", "插入换行"],
    ["文件输入", "Ctrl+O", "载入文本或表格文件"],
    ["计算", "Tab", "用当前数值继续计算"],
    ["计算", "Alt+F2", "切换普通计算与精确分数计算"],
    ["批量计算", "Alt+Enter", "选中出错的算式"],
    ["结果", "Enter", "复制结果并关闭搜索框"],
    ["结果", "F3", "填回结果或恢复填回前的查询"],
    ["精确分数", "Shift+F3", "填回预览的小数"],
    ["结果", "F5", "刷新或重新计算结果"],
    ["结果", "Ctrl+S", "保存结果到文件"],
    ["表格", "PageUp", "向上翻行页"],
    ["表格", "PageDown", "向下翻行页"],
    ["表格", "Ctrl+PageUp", "跳到第一组行"],
    ["表格", "Ctrl+PageDown", "跳到最后一组行"],
    ["表格", "Shift+PageUp", "向左翻列页"],
    ["表格", "Shift+PageDown", "向右翻列页"],
    ["表格", "Ctrl+Shift+PageUp", "跳到第一组列"],
    ["表格", "Ctrl+Shift+PageDown", "跳到最后一组列"],
    ["表格与图形", "F2", "在预览与原文数据间切换"],
    ["表格", "Ctrl+F", "按内容查找表格行"],
    ["表格", "Ctrl+Shift+C", "复制多页表格的当前页"],
    ["表格", "Ctrl+Shift+T", "复制完整表格为 TSV"],
    ["用法", "F1", "打开当前工具用法或返回原查询"],
    ["用法", "Tab", "填入选中的示例"],
    ["用法", "Shift+Tab", "填入示例并选中可替换参数"],
    ["用法", "Alt+Enter", "填入示例并选中可替换数据"],
    ["收藏", "F4", "打开收藏列表或返回原查询"],
    ["收藏", "Ctrl+D", "收藏或取消收藏查询"],
    ["收藏", "F2", "为选中的收藏命名"],
    ["收藏", "Alt+↑", "上移选中的收藏"],
    ["收藏", "Alt+↓", "下移选中的收藏"],
    ["查询历史", "F6", "打开查询历史或返回原查询"],
    ["查询历史", "Alt+Delete", "移除选中的历史记录"],
    ["算式历史", "Ctrl+R", "打开算式历史或返回原查询"],
    ["恢复查询", "Ctrl+Shift+R", "填回上次输入并恢复选区"],
    ["文件搜索", "Tab", "打开所选目录或补全目录查询"],
    ["文件搜索", "Alt+Enter", "在资源管理器中定位所选项"],
    ["文件搜索", "Alt+↑", "打开上一级目录"],
    ["文件搜索", "Alt+←", "返回上一个浏览的目录"],
    ["文件搜索", "Alt+→", "前往下一个浏览的目录"],
    ["文件搜索", "F3", "载入选中的文本文件"],
    ["文件搜索", "F5", "重新读取当前目录"],
    ["窗口", "Esc", "关闭搜索框"],
];

pub(super) fn shortcut_table(filter: &str) -> String {
    let filter = search_key(filter);
    let terms: Vec<_> = filter.split_whitespace().collect();
    let mut rows = vec![["场景", "按键", "操作"].map(str::to_owned)];
    for &shortcut in SHORTCUTS {
        if !terms.is_empty() {
            let searchable = search_key(&shortcut.join(" "));
            if !terms.iter().all(|&term| searchable.contains(term)) {
                continue;
            }
        }
        rows.push(shortcut.map(str::to_owned));
    }
    write_rows(&rows, ',')
}
