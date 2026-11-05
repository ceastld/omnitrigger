import type {ActionContext} from '../crates/omnitrigger-actions/runtime/api';
import closeTab from '../examples/actions/browser-close/close-tab.mjs';
import recipe from '../examples/actions/keyboard-sequence/recipe.mjs';
import {prepareRecipe} from '../examples/actions/keyboard-sequence/prepare-recipe.mjs';
import {runWorkflow} from '../examples/actions/browser-workflows/workflows.mjs';

declare const ctx: ActionContext;
// 对照当前正式 SDK，而不只检查各示例自己的声明副本。
const close = closeTab(ctx);
const keys = recipe(ctx);
const restore = runWorkflow(ctx, 'restore-session');
const plan = prepareRecipe(ctx.input);
void close; void keys; void restore;
// @ts-expect-error 纯准备结果不可改写，派发时复制成实际 SDK 接受的数组。
plan.steps.push({kind:'tap',key:65});

import sequence from '../examples/actions/keyboard-sequence/sequence.mjs';
import shortcuts from '../examples/actions/keyboard-sequence/shortcuts.mjs';
void sequence(ctx); void shortcuts(ctx);
