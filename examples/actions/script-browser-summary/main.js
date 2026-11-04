/// <reference path="../../../crates/omnitrigger-actions/runtime/script-api.d.ts" />
/** @typedef {{url?: string, children?: Bookmark[]}} Bookmark */
/** @param {ScriptContext} ctx */
function main(ctx) {
    const tree = /** @type {Bookmark[]} */ (ctx.browser.call('bookmarks.getTree', [], {
        ...(ctx.input.profile === undefined ? {} : {profile: ctx.input.profile}),
        timeoutMs: 10000,
    }));
    const pending = [...tree];
    let bookmarks = 0;
    let foldersIncludingRoots = 0;
    while (pending.length) {
        const node = pending.pop();
        if (!node) continue;
        if (typeof node.url === 'string') bookmarks++;
        else foldersIncludingRoots++;
        if (Array.isArray(node.children)) pending.push(...node.children);
    }
    return {bookmarks, foldersIncludingRoots};
}
