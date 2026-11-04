/// <reference path="../../../tools/js-engine-probe/api.d.ts" />
/** @param {ScriptContext} ctx */
function main(ctx) {
    const { source, destination } = ctx.input;
    if (typeof source !== 'string' || typeof destination !== 'string') {
        throw new Error('source 和 destination 必须是完整文件路径。');
    }
    const text = ctx.fs.readText(source);
    const cleaned = text.replace(/\r\n?/g, '\n').replace(/[\t ]+$/gm, '');
    ctx.fs.writeText(destination, cleaned);
    return { destination, characters: cleaned.length };
}
