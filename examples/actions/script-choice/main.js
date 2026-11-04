/** @type {ScriptMain} */
function main(ctx) {
    return ctx.ui.choose({
        title: '选择文本风格',
        items: [
            {id:'plain', label:'纯文本', description:'不附加格式'},
            {id:'markdown', label:'Markdown', description:'保留标题与列表'},
            {id:'code', label:'代码', description:'适合代码环境'},
        ],
    });
}
