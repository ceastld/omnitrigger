// 两类宿主共用的严格采集契约；invoke由宿主显式注入，不回退旧run或自行创建进程。
function createProcessCaptureApi(invoke, asyncMode = false) {
    if (typeof invoke !== 'function' || typeof asyncMode !== 'boolean') throw new TypeError('采集桥接配置无效。');
    const fail = code => { throw Object.assign(new Error('程序输出采集未完成，请检查错误代码。'), {code}); };
    const unicode = (value, nul = false) => {
        if (typeof value !== 'string' || (!nul && value.includes('\0'))) return false;
        for (let i = 0; i < value.length; i++) {
            const unit = value.charCodeAt(i);
            if (unit >= 0xd800 && unit <= 0xdbff) {
                const next = value.charCodeAt(++i);
                if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
            } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
        }
        return true;
    };
    const local = value => unicode(value) && /^[A-Za-z]:[\\/]/.test(value)
        && !value.slice(3).split(/[\\/]/).some(part => part === '.' || part === '..');
    function checked(value, allowNonZero) {
        if (!value || typeof value !== 'object' || Array.isArray(value)
            || Reflect.ownKeys(value).length !== 3 || !['code','stdout','stderr'].every(key => Object.hasOwn(value,key))
            || !Number.isInteger(value.code) || value.code < -2147483648 || value.code > 2147483647
            || value.stdout?.length > 65536 || value.stderr?.length > 65536
            || !unicode(value.stdout,true) || !unicode(value.stderr,true)) fail('invalid_response');
        let bytes = 0;
        for (const text of [value.stdout,value.stderr]) for (const character of text) {
            const point = character.codePointAt(0);
            bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
            if (bytes > 65536) fail('invalid_response');
        }
        if (value.code !== 0 && !allowNonZero) fail('process_exit_nonzero');
        return {code:value.code,stdout:value.stdout,stderr:value.stderr};
    }
    function execute(program, args = [], options = {}) {
        let dispatched = false;
        const rejected = cause => {
        let code = 'process_capture_failed';
        let effectState = dispatched ? 'unknown' : 'not_started';
        try {
            const known = ['invalid_input','invalid_arguments','invalid_response','request_too_large','unsupported_platform',
                'process_setup_failed','process_start_failed','process_read_failed','process_wait_failed','process_clock_error',
                'process_timeout','cancelled','timeout','native_bridge_unavailable','native_bridge_failed','desktop_busy','desktop_call_limit','desktop_connection','desktop_protocol','desktop_timeout','desktop_unavailable','process_output_too_large','process_invalid_utf8','process_exit_nonzero','process_cleanup_failed'];
            if (known.includes(cause?.code)) code = cause.code;
            if (cause?.effectState === 'not_started' || ['invalid_arguments','request_too_large','unsupported_platform',
                'process_setup_failed','process_start_failed'].includes(code)) effectState = 'not_started';
        } catch (_) { /* 不回显异常对象、getter、参数或采集输出。 */ }
        throw Object.assign(new Error('程序输出采集未完成，请检查错误代码。'), {code,effectState,method:'process.capture'});
        };
        try {
        if (typeof program !== 'string' || program.length > 32766 || !local(program) || !program.toLowerCase().endsWith('.exe') || !Array.isArray(args) || args.length > 128
            || Reflect.ownKeys(args).length !== args.length + 1 || args.some((arg, i) => !Object.hasOwn(args,i) || typeof arg !== 'string' || arg.length > 32766 || !unicode(arg))
            || !options || typeof options !== 'object' || Array.isArray(options)
            || Reflect.ownKeys(options).some(key => !['timeoutMs','allowNonZero','cwd'].includes(key))) fail('invalid_input');
        for (let i = 0; i < args.length; i++) if (!Object.hasOwn(args,i)) fail('invalid_input');
        if (program.length + args.reduce((length,arg) => length+arg.length,0) + 3*args.length + 3 > 32767) fail('invalid_input');
        const {timeoutMs = 10000, allowNonZero = false, cwd} = options;
        if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000 || typeof allowNonZero !== 'boolean'
            || (Object.hasOwn(options,'cwd') && !local(cwd))
            || Object.keys(options).some(key => options[key] === undefined)) fail('invalid_input');
        // 与本机 MSVCRT 引号规则计算同一命令行长度，不把参数拼给 shell。
        const quoted = value => {
            const parts = ['"'];
            let slashes = 0;
            for (const character of value) {
                if (character === '\\') { slashes++; continue; }
                parts.push('\\'.repeat(character === '"' ? slashes*2+1 : slashes), character);
                slashes = 0;
            }
            parts.push('\\'.repeat(slashes*2), '"');
            return parts.join('');
        };
        if ([program,...args].map(quoted).join(' ').length + 1 > 32767) fail('invalid_input');
            dispatched = true;
            const response = invoke({program,args:[...args],timeout_ms:timeoutMs,allow_non_zero:allowNonZero,
            ...(cwd === undefined ? {} : {cwd})});
            return asyncMode ? Promise.resolve(response).then(value => checked(value, allowNonZero)).catch(rejected) : checked(response, allowNonZero);
        } catch (cause) { return rejected(cause); }
    }
    return Object.freeze({capture: asyncMode ? async (...args) => execute(...args) : execute});
}
