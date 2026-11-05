import {prepareInput,verifyProfiles,verifyWindow,verifyTab,createReceipt,acknowledge,failedReceipt} from './close-common.mjs';

/** @type {ScriptMain} */
export function main(ctx) {
  const input = prepareInput(ctx.input);
  const options = Object.freeze({profile:input.profile,timeoutMs:input.timeoutMs});
  verifyProfiles(ctx.browser.profiles({timeoutMs:input.timeoutMs}), input);
  verifyWindow(ctx.browser.call('windows.get',[input.windowId],options), input);
  verifyTab(ctx.browser.call('tabs.get',[input.tabId],options), input);
  const receipt = createReceipt(input);
  try {
    ctx.browser.call('tabs.remove',[input.tabId],options);
    return acknowledge(receipt);
  } catch (error) {
    return failedReceipt(receipt,error);
  }
}
