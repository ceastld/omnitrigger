/** @typedef {'first_shortcut'|'inter_chord_delay'|'second_shortcut'} FoldPhase */

/** @param {unknown} input @returns {void} */
export function validateFoldInput(input) {
  if (!input || typeof input !== 'object' || Array.isArray(input) || Reflect.ownKeys(input).length !== 0) {
    throw Object.assign(new Error('折叠所有快捷键只接受空对象参数。'), {
      code: 'invalid_input', effectState: 'not_started', method: 'script.parameters',
    });
  }
}

/** @param {unknown} cause @param {number} accepted @param {FoldPhase} phase @returns {never} */
export function foldIncomplete(cause, accepted, phase) {
  const failure = /** @type {{code?:unknown,method?:unknown,effectState?:unknown}} */ (
    cause !== null && typeof cause === 'object' ? cause : {});
  const stepEffectState = failure.effectState === 'not_started' ? 'not_started' : 'unknown';
  const label = phase === 'first_shortcut' ? '首段按键' : phase === 'inter_chord_delay' ? '段间等待' : '末段按键';
  // 标准错误消息保留部分提交信息；不依赖宿主透传自定义属性。
  throw Object.assign(new Error('折叠序列未完整提交：已确认' + accepted + '段，' + label + '未确认。'), {
    code: typeof failure.code === 'string' ? failure.code : 'host_call_failed',
    method: typeof failure.method === 'string' ? failure.method : phase === 'inter_chord_delay' ? 'delay' : 'keyboard.shortcut',
    effectState: accepted > 0 ? 'unknown' : stepEffectState,
    stepEffectState, acceptedSubmissions: accepted, phase,
  });
}
