import type {ActionContext} from '../examples/actions/keyboard-sequence/sdk/api';
import recipe from '../examples/actions/keyboard-sequence/recipe.mjs';
import {prepareRecipe} from '../examples/actions/keyboard-sequence/prepare-recipe.mjs';
import sequence from '../examples/actions/keyboard-sequence/sequence.mjs';
import shortcuts from '../examples/actions/keyboard-sequence/shortcuts.mjs';

// 每个可携带动作包在自己的作者工程中解析 SDK，避免混入另一个包的全局声明和能力标记。
declare const ctx: ActionContext;
const keys = recipe(ctx);
const plan = prepareRecipe(ctx.input);
void keys; void sequence(ctx); void shortcuts(ctx);
// @ts-expect-error 纯准备结果不可改写，派发时复制成实际 SDK 接受的数组。
plan.steps.push({kind:'tap',key:65});
