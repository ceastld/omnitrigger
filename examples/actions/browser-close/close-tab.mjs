import {prepareInput,verifyProfiles,verifyWindow,verifyTab,createReceipt,acknowledge,failedReceipt} from './close-common.mjs';

/** @param {import('./sdk/api').ActionContext} ctx @returns {Promise<import('./close-common.mjs').CloseReceipt>} */
export default async function closeTab(ctx) {
  const input = prepareInput(ctx.input);
  const options = Object.freeze({profile:input.profile,timeoutMs:input.timeoutMs});
  verifyProfiles(await ctx.browser.profiles({timeoutMs:input.timeoutMs}), input);
  verifyWindow(await ctx.browser.call('windows.get',[input.windowId],options), input);
  verifyTab(await ctx.browser.call('tabs.get',[input.tabId],options), input);
  const receipt = createReceipt(input);
  try {
    await ctx.browser.call('tabs.remove',[input.tabId],options);
    return acknowledge(receipt);
  } catch (error) {
    return failedReceipt(receipt,error);
  }
}
