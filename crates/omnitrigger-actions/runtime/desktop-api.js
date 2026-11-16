// Node 与轻量宿主共用的桌面接口。传入桥接函数决定同步或异步执行方式。
function createDesktopApi(invoke) {
  function invalid(method, message) {
    throw Object.assign(new Error(message), {code:'invalid_input', effectState:'not_started', method});
  }
  function geometryFields(value, names) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error();
    const fields = Object.getOwnPropertyDescriptors(value);
    if (Reflect.ownKeys(fields).length !== names.length || names.some(name => !Object.hasOwn(fields, name) || !Object.hasOwn(fields[name], 'value'))) throw new Error();
    return Object.fromEntries(names.map(name => [name,fields[name].value]));
  }
  function geometryRect(value, request) {
    const rect = geometryFields(value, ['left','top','width','height']);
    if (!Object.values(rect).every(Number.isInteger) || rect.left < -2147483648 || rect.left > 2147483647
        || rect.top < -2147483648 || rect.top > 2147483647 || rect.width < 1 || rect.height < 1
        || rect.width > (request ? 65535 : 2147483647) || rect.height > (request ? 65535 : 2147483647)
        || rect.left + rect.width > 2147483647 || rect.top + rect.height > 2147483647) throw new Error();
    return rect;
  }
  function geometryResult(method, value) {
    try {
      if (method === 'window.set_rect') {
        const result = geometryFields(value,['submitted','effectState']);
        if (result.submitted !== true || result.effectState !== 'unknown') throw new Error();
        return result;
      }
      const result = geometryFields(value,['coordinateSpace','className','outerRect','visibleFrameRect','monitorRect','workArea','primaryScreen','windowDpi','maximized']);
      if (typeof result.className !== 'string' || !result.className.length || result.className.length > 255
          || result.className.includes('\0')) throw new Error();
      if (result.coordinateSpace !== 'physical-screen' || !Number.isInteger(result.windowDpi) || result.windowDpi < 1 || result.windowDpi > 65535 || typeof result.maximized !== 'boolean') throw new Error();
      for (let index = 0; index < result.className.length; index++) {
        const unit = result.className.charCodeAt(index);
        if (unit >= 0xd800 && unit <= 0xdbff) {
          const next = result.className.charCodeAt(++index);
          if (!(next >= 0xdc00 && next <= 0xdfff)) throw new Error();
        } else if (unit >= 0xdc00 && unit <= 0xdfff) throw new Error();
      }
      for (const name of ['outerRect','visibleFrameRect','monitorRect','workArea']) result[name] = geometryRect(result[name],false);
      result.primaryScreen = geometryFields(result.primaryScreen,['width','height']);
      if (!Object.values(result.primaryScreen).every(n => Number.isInteger(n) && n > 0 && n <= 2147483647)) throw new Error();
      return result;
    } catch (_) {
      throw Object.assign(new Error('窗口矩形回执无效；不会自动重试。'),{code:'invalid_response',method,effectState:method === 'window.geometry' ? 'not_started' : 'unknown'});
    }
  }
  function geometryCall(method, params) {
    const result = invoke(method,params);
    return result && typeof result.then === 'function' ? result.then(value => geometryResult(method,value)) : geometryResult(method,result);
  }
  function trackingResult(method,value) {
    try {
      if (method === 'pointer.state') {
        const result=geometryFields(value,['coordinateSpace','x','y','buttons','inputSource']);
        if (result.coordinateSpace!=='physical-screen'
            || ![result.x,result.y].every(n=>Number.isInteger(n)&&n>=-2147483648&&n<=2147483647)) throw new Error();
        result.inputSource=geometryFields(result.inputSource,['kind','injected','selfInjected']);
        if (result.inputSource.kind!=='polled-key-state' || result.inputSource.injected!==null || result.inputSource.selfInjected!==null) throw new Error();
        result.buttons=geometryFields(result.buttons,['left','right','middle']);
        if (!Object.values(result.buttons).every(v=>['down','not_down_or_unavailable'].includes(v))) throw new Error();
        return result;
      }
      const result=geometryFields(value,['stopped','reason','acceptedUpdates','pollSamples','effectState','dragCompleted']);
      if (result.stopped!==true || result.dragCompleted!==false || !['button_not_down_or_unavailable','duration_limit','update_limit'].includes(result.reason)
          || !Number.isInteger(result.acceptedUpdates) || result.acceptedUpdates<0 || result.acceptedUpdates>64
          || !Number.isInteger(result.pollSamples) || result.pollSamples<0 || result.pollSamples>201
          || !['not_started','unknown'].includes(result.effectState) || (result.acceptedUpdates>0 && result.effectState!=='unknown')) throw new Error();
      return result;
    } catch (_) { throw Object.assign(new Error('指针/有界调整回执无效；不会重试。'),{code:'invalid_response',method,effectState:method==='pointer.state'?'not_started':'unknown'}); }
  }
  function trackingCall(method,params) {
    const result=invoke(method,params);
    return result && typeof result.then==='function' ? result.then(value=>trackingResult(method,value)) : trackingResult(method,result);
  }
  // 鼠标按钮VK不是键盘输入；真实鼠标按下/释放需要独立的所有权与收尾协议。
  function keyboardVirtualKey(key) {
    return Number.isInteger(key) && key >= 1 && key <= 254 && ![1,2,4,5,6].includes(key);
  }
  function keyTiming(options, count, method) {
    const value = options?.keyDownDelayMs === undefined ? 0 : options.keyDownDelayMs;
    if (!options || typeof options !== 'object' || Array.isArray(options) || Object.keys(options).some(key => key !== 'keyDownDelayMs')
        || !Number.isInteger(value) || value < 0 || value > 1000 || (count + 1) * value > 3000) {
      invalid(method, '按下延迟需要 0–1000ms 整数，单次等待合计不能超过 3000ms。');
    }
    return value;
  }
  function textCall(method, text, extra = {}) {
    let valid = typeof text === 'string' && text.length <= 65536 && !text.includes('\0');
    if (valid) for (let i = 0; i < text.length; i++) {
      const unit = text.charCodeAt(i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = text.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) { valid = false; break; }
      } else if (unit >= 0xdc00 && unit <= 0xdfff) { valid = false; break; }
    }
    if (!valid) invalid(method, '文本需要有效 Unicode，不含 NUL，最多 65,536 个 UTF-16 单元。');
    return invoke(method, {text, ...extra});
  }
  function dataFault(method, code, effectState) {
    return Object.assign(new Error('剪贴板或选区数据调用失败；请检查错误代码。'), {code, effectState, method});
  }
  function dataText(value, max, allowEmpty) {
    if (typeof value !== 'string' || value.length > max || (!allowEmpty && value.length === 0) || value.includes('\0')) return false;
    for (let i = 0; i < value.length; i++) {
      const unit = value.charCodeAt(i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = value.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
      } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
    }
    return true;
  }
  function exactDataFields(value, fields) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      && Reflect.ownKeys(value).length === fields.length && fields.every(key => Object.hasOwn(value, key));
  }
  function dataCall(method, params) {
    const copy = method === 'selection.copy_data';
    const effect = copy ? 'unknown' : 'not_started';
    const failure = error => {
      let code = 'native_bridge_failed';
      let notStarted = !copy;
      try {
        const reported = error?.code;
        if (typeof reported === 'string' && /^[A-Za-z0-9_]{1,64}$/.test(reported)) code = reported;
        if (copy) notStarted = error?.effectState === 'not_started';
      } catch (_) { /* 不读取或回显异常 getter 的正文。 */ }
      throw dataFault(method, code, notStarted ? 'not_started' : 'unknown');
    };
    const result = value => {
      try {
        const kind = value?.kind;
        const files = kind === 'files';
        const fields = ['kind', files ? 'paths' : 'text', 'sequenceNumber', ...(copy ? ['changed'] : [])];
        if (!exactDataFields(value, fields) || !['text','files'].includes(kind)
            || (copy && params.format !== undefined && params.format !== kind)) throw new Error();
        const sequenceNumber = value.sequenceNumber;
        const changed = copy ? value.changed : undefined;
        if (!Number.isInteger(sequenceNumber) || sequenceNumber < 0 || sequenceNumber > 0xffffffff
            || (copy && typeof changed !== 'boolean')) throw new Error();
        let data;
        if (files) {
          const source = value.paths;
          if (!Array.isArray(source) || source.length > 64
              || Reflect.ownKeys(source).length !== source.length + 1) throw new Error();
          const paths = [];
          let units = 0;
          for (let i = 0; i < source.length; i++) {
            if (!Object.hasOwn(source, i)) throw new Error();
            const path = source[i];
            if (!dataText(path, 65536 - units, false)) throw new Error();
            units += path.length;
            paths.push(path);
          }
          // 路径仅复制数据，不套用 stat、云盘、绝对路径或存在性规则，不 trim。
          data = {kind:'files', paths, sequenceNumber};
        } else {
          const text = value.text;
          if (!dataText(text, 65536, true)) throw new Error();
          data = {kind:'text', text, sequenceNumber};
        }
        return copy ? {...data, changed} : data;
      } catch (_) { throw dataFault(method, 'invalid_response', effect); }
    };
    try {
      const value = invoke(method, params);
      return value && typeof value.then === 'function' ? value.then(result, failure) : result(value);
    } catch (error) { return failure(error); }
  }
  function brightnessFault(code, effectState) {
    return Object.assign(new Error('亮度步进未获完整确认；请检查结构化回执。'),
      {code, effectState, method:'display.brightness_step'});
  }
  function brightnessRecord(value, fields) {
    if (!exactDataFields(value, fields)) throw new Error();
    const result = {};
    for (const key of fields) {
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (!descriptor || !Object.hasOwn(descriptor, 'value')) throw new Error();
      result[key] = descriptor.value;
    }
    return result;
  }
  function brightnessResponse(value, strategy) {
    const codes = ['initialization_failed','monitor_unavailable','ambiguous_monitor',
      'brightness_unavailable','brightness_failed','brightness_unknown'];
    const uint = (value,max) => Number.isInteger(value) && value >= 0 && value <= max;
    const errorCode = code => code === null || codes.includes(code);
    const physical = value => {
      const report = brightnessRecord(value, ['members','errorCode']);
      const items = report.members;
      if (!errorCode(report.errorCode) || !Array.isArray(items) || items.length > 64
          || Reflect.ownKeys(items).length !== items.length + 1
          || (report.errorCode === null ? items.length === 0 : items.length !== 0)) throw new Error();
      const members = [];
      for (let index = 0; index < items.length; index++) {
        const descriptor = Object.getOwnPropertyDescriptor(items, String(index));
        if (!descriptor || !Object.hasOwn(descriptor, 'value')) throw new Error();
        const row = brightnessRecord(descriptor.value,
          ['index','status','previousRaw','requestedRaw','currentRaw','errorCode']);
        if (row.index !== index || !errorCode(row.errorCode)) throw new Error();
        const previous = uint(row.previousRaw,0xffffffff), requested = uint(row.requestedRaw,0xffffffff);
        const current = uint(row.currentRaw,0xffffffff);
        switch (row.status) {
          case 'read_rejected':
            if (row.previousRaw !== null || row.requestedRaw !== null || row.currentRaw !== null || row.errorCode === null) throw new Error(); break;
          case 'unchanged':
            if (!previous || !requested || !current || row.previousRaw !== row.requestedRaw || row.currentRaw !== row.requestedRaw || row.errorCode !== null) throw new Error(); break;
          case 'set_unconfirmed': case 'readback_unknown':
            if (!previous || !requested || row.currentRaw !== null || row.errorCode === null) throw new Error(); break;
          case 'readback_different': case 'confirmed':
            if (!previous || !requested || !current || row.errorCode !== null
                || (row.status === 'confirmed' && row.currentRaw !== row.requestedRaw)) throw new Error(); break;
          default: throw new Error();
        }
        members.push(row);
      }
      return {members,errorCode:report.errorCode};
    };
    const wmi = value => {
      const row = brightnessRecord(value,['status','previousPercent','requestedPercent','currentPercent','errorCode']);
      if (!errorCode(row.errorCode)) throw new Error();
      if (row.status === 'failed') {
        if (row.errorCode === null || row.previousPercent !== null || row.requestedPercent !== null || row.currentPercent !== null) throw new Error();
      } else {
        if (row.errorCode !== null || !uint(row.previousPercent,100) || !uint(row.currentPercent,100)) throw new Error();
        if (row.status === 'set') {
          if (!uint(row.requestedPercent,100) || row.requestedPercent === 0) throw new Error();
        } else if (!['zero_skipped','no_eligible_level'].includes(row.status) || row.requestedPercent !== null
            || row.previousPercent !== row.currentPercent) throw new Error();
      }
      return row;
    };
    const report = brightnessRecord(value,['strategy','effectState','physical','wmi']);
    if (report.strategy !== strategy || !['not_started','unknown'].includes(report.effectState)) throw new Error();
    if ((strategy === 'physical' && (report.physical === null || report.wmi !== null))
        || (strategy === 'wmi_first' && (report.physical !== null || report.wmi === null))
        || (strategy === 'physical_then_wmi' && report.physical === null)) throw new Error();
    const result = {strategy,effectState:report.effectState,physical:report.physical === null ? null : physical(report.physical),
      wmi:report.wmi === null ? null : wmi(report.wmi)};
    if (result.effectState === 'not_started' && (result.physical?.members.some(row => !['read_rejected','unchanged'].includes(row.status))
        || result.wmi?.status === 'set' || result.physical?.errorCode === 'brightness_unknown'
        || result.wmi?.errorCode === 'brightness_unknown')) throw new Error();
    if (strategy === 'physical_then_wmi' && result.wmi !== null) {
      const ddc = result.physical;
      if (ddc.errorCode !== null
          ? !['monitor_unavailable','brightness_unavailable'].includes(ddc.errorCode)
          : !ddc.members.every(row => row.status === 'read_rejected')) throw new Error();
    }
    return result;
  }
  function brightnessStep(args) {
    const method = 'display.brightness_step';
    let params;
    try {
      if (args.length !== 1) throw new Error();
      params = brightnessRecord(args[0],['direction','strategy']);
      if (!['up','down'].includes(params.direction)
          || !['physical','wmi_first','physical_then_wmi'].includes(params.strategy)) throw new Error();
    } catch (_) { throw brightnessFault('invalid_input','not_started'); }
    const requestedStrategy = params.strategy;
    const failure = cause => {
      const allowed = ['invalid_input','cancelled','timeout','native_bridge_unavailable','native_bridge_failed',
        'desktop_unavailable','desktop_busy','desktop_call_limit','desktop_connection','desktop_protocol','desktop_timeout',
        'invalid_response','initialization_failed','monitor_unavailable','ambiguous_monitor','brightness_unavailable',
        'brightness_failed','brightness_unknown'];
      let failure;
      try { desktopMutationFailure(cause,allowed,brightnessFault); }
      catch (error) { failure=error; }
      // 只接纳当前策略、unknown 状态且通过现有匿名回执校验的数据。
      if (['cancelled','timeout'].includes(failure.code) && failure.effectState==='unknown') {
        try {
          const receipt=Object.getOwnPropertyDescriptor(cause,'brightnessReceipt')?.value;
          const reportedMethod=Object.getOwnPropertyDescriptor(cause,'method')?.value;
          const reportedEffect=Object.getOwnPropertyDescriptor(cause,'effectState')?.value;
          if (reportedMethod===method && reportedEffect==='unknown' && receipt!==undefined) {
            const validated=brightnessResponse(receipt,requestedStrategy);
            if (validated.effectState==='unknown') failure.brightnessReceipt=validated;
          }
        } catch (_) { /* 无效进度不能覆盖原取消/超时，也不能变为重试证明。 */ }
      }
      throw failure;
    };
    const result = value => {
      try { return brightnessResponse(value,requestedStrategy); }
      catch (_) { throw brightnessFault('invalid_response','unknown'); }
    };
    try {
      const value = invoke(method,params);
      return value && typeof value.then === 'function' ? value.then(result,failure) : result(value);
    } catch (cause) { return failure(cause); }
  }

  function desktopMutationFailure(cause, allowed, makeFault) {
    let code='native_bridge_failed',effect='unknown';
    try {
      const reported=cause != null ? Object.getOwnPropertyDescriptor(cause,'code')?.value : undefined;
      const reportedEffect=cause != null ? Object.getOwnPropertyDescriptor(cause,'effectState')?.value : undefined;
      if(allowed.includes(reported))code=reported;
      if(reportedEffect==='not_started')effect='not_started';
    } catch (_) { /* 不读取或回显异常正文。 */ }
    throw makeFault(code,effect);
  }
  function volumeFault(code,effectState){
    return Object.assign(new Error('音量调节未获完整确认；请检查错误代码和效果状态。'),
      {code,effectState:code==='audio_unknown'?'unknown':effectState,method:'audio.volume_adjust'});
  }
  function adjustVolume(args){
    let delta;
    try{
      if(args.length!==1)throw new Error();
      const params=brightnessRecord(args[0],['deltaPercent']);
      delta=params.deltaPercent;
      if(!Number.isInteger(delta)||delta===0||delta < -100||delta > 100)throw new Error();
    }catch(_){throw volumeFault('invalid_input','not_started');}
    const failure=cause=>desktopMutationFailure(cause,
      ['invalid_input','cancelled','timeout','native_bridge_unavailable','native_bridge_failed','desktop_unavailable',
        'desktop_busy','desktop_call_limit','desktop_connection','desktop_protocol','desktop_timeout','invalid_response',
        'initialization_failed','audio_unavailable','audio_failed','audio_unknown'],volumeFault);
    const result=value=>{
      try{
        const row=brightnessRecord(value,['effectState','previousPercent','requestedPercent','currentPercent']);
        if(row.effectState!=='unknown'||![row.previousPercent,row.requestedPercent,row.currentPercent].every(value=>Number.isInteger(value)&&value>=0&&value<=100)
          ||(delta>0&&row.requestedPercent<row.previousPercent)||(delta<0&&row.requestedPercent>row.previousPercent))throw new Error();
        return row;
      }catch(_){throw volumeFault('invalid_response','unknown');}
    };
    try{
      const value=invoke('audio.volume_adjust',{delta_percent:delta});
      return value&&typeof value.then==='function'?value.then(result,failure):result(value);
    }catch(cause){return failure(cause);}
  }

  function pointingObservation(value, method, code, effectState) {
    const identity=value=>{
      const row=geometryFields(value,['window_handle','process_id','thread_id','process_created_file_time']);
      if(!scopedDecimal(row.window_handle,'9223372036854775807')
        || ![row.process_id,row.thread_id].every(value=>Number.isInteger(value)&&value>0&&value<=0xffffffff)
        || !scopedDecimal(row.process_created_file_time,'18446744073709551615')) throw new Error();
      return Object.freeze(row);
    };

      try{
        const row=geometryFields(value,['sampledAt','coordinateSpace','point','inputSource','pointingWindow','rootWindow']);
        const point=geometryFields(row.point,['x','y']);
        const source=geometryFields(row.inputSource,['kind','injected','selfInjected']);
        if(row.sampledAt!=='call-time'||row.coordinateSpace!=='physical-screen'
          ||![point.x,point.y].every(value=>Number.isInteger(value)&&value>=-2147483648&&value<=2147483647)
          ||source.kind!=='polled-pointer'||source.injected!==null||source.selfInjected!==null) throw new Error();
        return Object.freeze({...row,point:Object.freeze(point),inputSource:Object.freeze(source),
          pointingWindow:identity(row.pointingWindow),rootWindow:identity(row.rootWindow)});
      }catch(_){throw Object.assign(new Error('指向根窗观察回执无效。'),{code,method,effectState});}
  }

  function pointingRoot(args) {
    const method='pointer.root';
    if(args.length!==0) invalid(method,'getPointingRoot 不接收参数、坐标或窗口句柄。');
    const failure=cause=>{
      const code=typeof cause?.code==='string'&&/^[a-z_]{1,64}$/.test(cause.code)?cause.code:'native_bridge_failed';
      throw Object.assign(new Error('指向根窗只读查询未获确认。'),{code,method,effectState:'not_started'});
    };
    const result=value=>pointingObservation(value,method,'invalid_response','not_started');
    try{const value=invoke(method,{});return value&&typeof value.then==='function'?value.then(result,failure):result(value);}
    catch(cause){return failure(cause);}
  }


  function handoffPointingRoot(args) {
    const method='target.handoff_pointing_root';
    let expected,options;
    try{
      if(args.length!==2) throw new Error();
      expected=pointingObservation(args[0],method,'invalid_input','not_started');
      options=geometryFields(args[1],['validForMs','confirmTimeoutMs']);
      if(!Number.isInteger(options.validForMs)||options.validForMs<2||options.validForMs>5000
        ||!Number.isInteger(options.confirmTimeoutMs)||options.confirmTimeoutMs<1||options.confirmTimeoutMs>1000
        ||options.confirmTimeoutMs>=options.validForMs) throw new Error();
    }catch(_){invalid(method,'交接需要完整指向观察及显式有效期/确认预算；确认预算须短于有效期。');}
    const failure=cause=>{
      const code=typeof cause?.code==='string'&&/^[a-z_]{1,64}$/.test(cause.code)?cause.code:'native_bridge_failed';
      throw Object.assign(new Error('指向根窗交接未获确认；不会自动重试。'),{code,method,effectState:cause?.effectState==='not_started'?'not_started':'unknown'});
    };
    const result=value=>{
      try{
        const row=geometryFields(value,['requested','requestAccepted','foregroundConfirmed','target','reason','effectState']);
        if(row.requested!==true||typeof row.requestAccepted!=='boolean'||typeof row.foregroundConfirmed!=='boolean'||row.effectState!=='unknown') throw new Error();
        if(!row.requestAccepted){
          if(row.reason!=='os_rejected'||row.target!==null) throw new Error();
        }else if(!row.foregroundConfirmed){
          if(row.reason!=='confirmation_timeout'||row.target!==null) throw new Error();
        }else{
          if(row.reason!==null) throw new Error();
          const target=scopedTarget(row.target,method,'invalid_response');
          for(const field of ['window_handle','process_id','thread_id','process_created_file_time'])
            if(target.context[field]!==expected.rootWindow[field]) throw new Error();
          row.target=target;
        }
        return Object.freeze(row);
      }catch(_){throw Object.assign(new Error('交接回执无效，不能据此发送输入。'),{code:'invalid_response',method,effectState:'unknown'});}
    };
    const params={expected_root:{...expected.rootWindow},valid_for_ms:options.validForMs,confirm_timeout_ms:options.confirmTimeoutMs};
    try{const value=invoke(method,params);return value&&typeof value.then==='function'?value.then(result,failure):result(value);}
    catch(cause){return failure(cause);}
  }


  function windowDragParams(args) {
      const method='window.drag'; let params;
      try {
        if (args.length!==1 || !args[0] || typeof args[0]!=='object') throw new Error();
        const names=Reflect.ownKeys(args[0]);
        if (names.some(n=>!['mode','button','maxDurationMs','maxUpdates','snap','initialPointer'].includes(n))) throw new Error();
        const value=geometryFields(args[0],names);
        if (!['move','resize'].includes(value.mode) || !['left','right','middle'].includes(value.button)
            || !Number.isInteger(value.maxDurationMs) || value.maxDurationMs<20 || value.maxDurationMs>2000
            || !Number.isInteger(value.maxUpdates) || value.maxUpdates<1 || value.maxUpdates>64) throw new Error();
        params={mode:value.mode,button:value.button,max_duration_ms:value.maxDurationMs,max_updates:value.maxUpdates};
        if (value.initialPointer!==undefined) {
          const point=geometryFields(value.initialPointer,['x','y']);
          if (![point.x,point.y].every(n=>Number.isInteger(n)&&n>=-2147483648&&n<=2147483647)) throw new Error();
          params.initial_pointer={x:point.x,y:point.y};
        }
        if (value.snap!==undefined && value.snap!==null) {
          const snap=geometryFields(value.snap,['bounds','distance','waitMs','initialElapsedMs']);
          if (value.mode!=='move' || !Number.isInteger(snap.distance) || snap.distance<0 || snap.distance>100
              || !Number.isInteger(snap.waitMs) || snap.waitMs<0 || snap.waitMs>1000
              || !Number.isInteger(snap.initialElapsedMs) || snap.initialElapsedMs<0 || snap.initialElapsedMs>60000) throw new Error();
          params.snap={bounds:geometryRect(snap.bounds,true),distance:snap.distance,wait_ms:snap.waitMs,initial_elapsed_ms:snap.initialElapsedMs};
        }
      } catch (_) { invalid(method,'有界调整需要明确mode/button、20–2000ms、1–64次预算；吸附需完整的显式物理边界/距离/时间参数。'); }
      return params;
  }

  function windowRectParams(args) {
      const method = 'window.set_rect';
      let rect, frame;
      try {
        if (args.length !== 2) throw new Error();
        rect = geometryRect(args[0],true);
        frame = geometryFields(args[1],['frame']).frame;
        if (!['outer','visible'].includes(frame)) throw new Error();
      } catch (_) { invalid(method,'矩形需要明确left/top/width/height整数及frame=outer或visible；宽高1–65535且边界不溢出。'); }
      return {rect,frame};
  }

function capturedWindow(args, count, method, buildParams, validate=value=>value) {
    if(args.length!==count) invalid(method,'显式窗口接口的参数数量无效。');
    const target=scopedTarget(args[0],method);
    const failure=cause=>{
      let code=typeof cause?.code==='string'&&/^[a-z_]{1,64}$/.test(cause.code)?cause.code:'native_bridge_failed';
      // 专用类名拒绝只有同一读取方法的not_started才有证明力；不把坏字段正规化成可静默拒绝。
      if(code==='window_class_blocked'&&(cause?.method!==method||cause?.effectState!=='not_started'))code='invalid_response';
      throw Object.assign(new Error('显式窗口租约调用未获确认；不会自动重试。'),{code,method,
        effectState:method==='window.geometry'||cause?.effectState==='not_started'?'not_started':'unknown'});
    };
    // 复用同一参数/回执解码，直接携带固定target调用；不在同步业务栈中重建整套SDK。
    try{const value=invoke(method,{...buildParams(),target});return value&&typeof value.then==='function'?value.then(validate).catch(failure):validate(value);}
    catch(cause){return failure(cause);}
  }

  function scopedFault(method, code, effectState) {
    return Object.assign(new Error('显式目标调用未获确认；请检查错误代码。'), {method, code, effectState});
  }
  function scopedRecord(value, fields) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      && Reflect.ownKeys(value).length === fields.length && fields.every(key => Object.hasOwn(value, key));
  }
  function scopedDecimal(value, max) {
    return typeof value === 'string' && /^[1-9][0-9]*$/.test(value)
      && (value.length < max.length || (value.length === max.length && value <= max));
  }
  function scopedTarget(value, method, code = 'invalid_input') {
    const context = value?.context;
    if (!scopedRecord(value, ['capture_id', 'context'])
        || !Number.isInteger(value.capture_id) || value.capture_id < 1 || value.capture_id > 128
        || !scopedRecord(context, ['window_handle','process_id','thread_id','process_created_file_time','expires_at_unix_ms'])
        || !scopedDecimal(context.window_handle, '9223372036854775807')
        || ![context.process_id,context.thread_id].every(n => Number.isInteger(n) && n > 0 && n <= 0xffffffff)
        || !scopedDecimal(context.process_created_file_time, '18446744073709551615')
        || !Number.isSafeInteger(context.expires_at_unix_ms) || context.expires_at_unix_ms <= 0) {
      throw scopedFault(method, code, 'not_started');
    }
    return Object.freeze({capture_id:value.capture_id, context:Object.freeze({...context})});
  }
  function scopedResult(operation, method, result) {
    const failure = cause => {
      const code = typeof cause?.code === 'string' && /^[a-z_]{1,64}$/.test(cause.code) ? cause.code : 'native_bridge_failed';
      throw scopedFault(method, code, method === 'target.capture_foreground' || cause?.effectState === 'not_started' ? 'not_started' : 'unknown');
    };
    try { const value = operation(); return value && typeof value.then === 'function' ? value.then(result, failure) : result(value); }
    catch (cause) { return failure(cause); }
  }
  function captureForegroundTarget(options) {
    const method = 'target.capture_foreground';
    if (!scopedRecord(options, ['expectedProcessId','expectedProcessCreatedFileTime','validForMs'])
        || !Number.isInteger(options.expectedProcessId) || options.expectedProcessId < 1 || options.expectedProcessId > 0xffffffff
        || !scopedDecimal(options.expectedProcessCreatedFileTime, '18446744073709551615')
        || !Number.isInteger(options.validForMs) || options.validForMs < 1 || options.validForMs > 5000) {
      throw scopedFault(method, 'invalid_input', 'not_started');
    }
    const params = {expected_process_id:options.expectedProcessId,
      expected_process_created_file_time:options.expectedProcessCreatedFileTime, valid_for_ms:options.validForMs};
    const result = value => {
      const target = scopedTarget(value, method, 'invalid_response');
      if (target.context.process_id !== params.expected_process_id
          || target.context.process_created_file_time !== params.expected_process_created_file_time) {
        throw scopedFault(method, 'invalid_response', 'not_started');
      }
      return target;
    };
    return scopedResult(() => invoke(method, params), method, result);
  }
  function scopedInput(method, target, call) {
    const stableTarget = scopedTarget(target, method);
    const api = createDesktopApi((_innerMethod, input) => {
      const result = value => {
        if (!scopedRecord(value, ['submitted']) || value.submitted !== true) throw scopedFault(method, 'invalid_response', 'unknown');
        return {submitted:true};
      };
      return scopedResult(() => invoke(method, {target:stableTarget, input}), method, result);
    });
    try { return call(api.desktop); }
    catch (cause) { if (cause?.code === 'invalid_input') cause.method = method; throw cause; }
  }

  // 默认128/reject；显式界限最多8192。只接受精确UTF16前缀，不修复拆开的代理项。
  function readSelectionText(args) {
    const method = 'selection.read_text';
    const fault = code => Object.assign(new Error('只读选区读取未获确认；请检查错误代码。'), {code,method,effectState:'not_started'});
    let overflow, maxUtf16Units, params;
    try {
      const options = args[0] === undefined ? {} : args[0];
      if (args.length > 1 || options === null || typeof options !== 'object' || Array.isArray(options)
          || Reflect.ownKeys(options).some(key => key !== 'overflow' && key !== 'maxUtf16Units')) throw fault('invalid_input');
      const mode = options.overflow, limit = options.maxUtf16Units;
      overflow = mode === undefined ? 'reject' : mode;
      maxUtf16Units = limit === undefined ? 128 : limit;
      if (!['reject','truncate'].includes(overflow) || !Number.isInteger(maxUtf16Units) || maxUtf16Units < 1 || maxUtf16Units > 8192) throw fault('invalid_input');
      params = mode === undefined ? {} : {overflow};
      if (limit !== undefined) params.max_utf16_units = maxUtf16Units;
    } catch (_) { throw fault('invalid_input'); }
    const result = value => {
      if (!exactDataFields(value,['text','truncated']) || !dataText(value.text,maxUtf16Units,true)
          || typeof value.truncated !== 'boolean' || value.truncated && (overflow !== 'truncate' || value.text.length !== maxUtf16Units)) throw fault('invalid_response');
      const snapshot = {text:value.text,truncated:value.truncated};
      let bytes = 0;
      for (const character of JSON.stringify(snapshot)) {
        const point = character.codePointAt(0);
        bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      }
      if (bytes > 6*maxUtf16Units+64 || bytes > 64*1024) throw fault('invalid_response');
      return Object.freeze(snapshot);
    };
    const failure = cause => {
      const code = typeof cause?.code === 'string' && /^[a-z_]{1,64}$/.test(cause.code) ? cause.code : 'native_bridge_failed';
      throw fault(code);
    };
    try {
      const value = invoke(method,params);
      return value && typeof value.then === 'function' ? value.then(result,failure) : result(value);
    } catch (cause) { return failure(cause); }
  }



  // 只读取本次原目标；不接受 HWND、前台重捕获、复制或截断选项。
  function explorerContext(args) {
    const method = 'explorer.context';
    const fault = code => Object.assign(new Error('原资源管理器上下文读取未获确认；请检查错误代码。'), {code,method,effectState:'not_started'});
    let selectionFailureStage = false;
    if (args.length !== 0) {
      try {
        const options = args[0];
        if (args.length !== 1 || !options || typeof options !== 'object' || Array.isArray(options)) throw new Error();
        const keys = Reflect.ownKeys(options);
        const descriptor = Object.getOwnPropertyDescriptor(options, 'selectionFailureStage');
        if (keys.length !== 1 || keys[0] !== 'selectionFailureStage' || !descriptor
            || !Object.hasOwn(descriptor, 'value') || descriptor.value !== true) throw new Error();
        selectionFailureStage = true;
      } catch (_) { throw fault('invalid_input'); }
    }
    const validPath = path => {
      if (!dataText(path,1024,false) || /[\x00-\x1f<>"|?*]/.test(path)) return false;
      const local = /^[A-Za-z]:\\/.test(path) && !path.slice(2).includes(':');
      const unc = /^\\\\[^\\/:]+\\[^\\/:]+(?:\\|$)/.test(path) && !path.includes(':') && !/^\\\\[?.]\\/.test(path);
      return (local || unc) && !path.split('\\').some(part => part === '.' || part === '..');
    };
    const result = value => {
      if (!exactDataFields(value,['currentDirectory','selectedPaths']) || !(value.currentDirectory === null || validPath(value.currentDirectory))
          || !Array.isArray(value.selectedPaths) || value.selectedPaths.length > 32
          || Object.keys(value.selectedPaths).length !== value.selectedPaths.length
          || value.selectedPaths.some(path => !validPath(path))
          || value.selectedPaths.reduce((n,path) => n+path.length,value.currentDirectory?.length ?? 0) > 8192
          || new Set(value.selectedPaths.map(path => path.toLowerCase())).size !== value.selectedPaths.length) throw fault('invalid_response');
      return Object.freeze({currentDirectory:value.currentDirectory,selectedPaths:Object.freeze([...value.selectedPaths])});
    };
    const failure = cause => {
      if (selectionFailureStage && cause?.code === 'selection_unavailable_after_directory') {
        // 只转交Native给出的证明错误；不把未知效果/缺失method升级为可复制证据。
        throw Object.assign(fault('selection_unavailable_after_directory'), {method:cause.method,effectState:cause.effectState});
      }
      throw fault(typeof cause?.code === 'string' && /^[a-z_]{1,64}$/.test(cause.code) ? cause.code : 'native_bridge_failed');
    };
    try {
      const value = invoke(method,selectionFailureStage ? {selection_failure_stage:true} : {});
      return value && typeof value.then === 'function' ? value.then(result,failure) : result(value);
    } catch(cause) { return failure(cause); }
  }

  function editFault(code,effectState) {
    return Object.assign(new Error('Win32 Edit选区替换未获完整确认；请检查错误代码和效果状态。'),
      {code,effectState,method:'win32_edit.replace_selection'});
  }
  function win32EditReplaceSelection(args) {
    let text,adapter;
    try {
      if(args.length!==2 || !dataText(args[0],8192,true))throw new Error();
      text=args[0];adapter=brightnessRecord(args[1],['adapter']).adapter;
      if(!['native-edit','winforms-edit'].includes(adapter))throw new Error();
    } catch (_) { throw editFault('invalid_input','not_started'); }
    const failure = cause => desktopMutationFailure(cause,
      ['invalid_input','cancelled','timeout','target_changed','edit_unsupported','permission_denied',
       'edit_message_unconfirmed','native_bridge_unavailable','native_bridge_failed','desktop_unavailable',
       'desktop_busy','desktop_call_limit','desktop_connection','desktop_protocol','desktop_timeout','invalid_response'],editFault);
    const result = value => {
      try {
        const receipt=brightnessRecord(value,['messageProcessed','effectState']);
        if(receipt.messageProcessed!==true || receipt.effectState!=='unknown')throw new Error();
        return Object.freeze({messageProcessed:true,effectState:'unknown'});
      } catch (_) { throw editFault('invalid_response','unknown'); }
    };
    try {
      const value=invoke('win32_edit.replace_selection',{text,adapter});
      return value && typeof value.then==='function' ? value.then(result,failure) : result(value);
    } catch (cause) { return failure(cause); }
  }

  const raiResidents = new WeakSet();
  function raiFault(method, code, effectState) {
    return Object.assign(new Error('RaiShot 常驻请求未获完整确认；请检查错误代码和效果状态。'),
      {code,method,effectState,retryAllowed:false});
  }
  function raiCall(method, params, result) {
    const failed = cause => {
      let code='native_bridge_failed',effect=method==='raishot.resident.find'?'not_started':'unknown';
      try {
        if(typeof cause?.code==='string'&&/^[a-z_]{1,64}$/.test(cause.code))code=cause.code;
        if(cause?.effectState==='not_started')effect='not_started';
      } catch (_) { /* 不读取异常正文或回显失败getter。 */ }
      throw raiFault(method,code,effect);
    };
    try {
      const value = invoke(method,params);
      return value && typeof value.then === 'function' ? value.then(result,failed) : result(value);
    } catch(cause) { return failed(cause); }
  }
  function findRaiShotResident(args) {
    const method='raishot.resident.find';
    if(args.length)throw raiFault(method,'invalid_input','not_started');
    return raiCall(method,{},value=>{
      if(value===null)return null;
      const decimal=(v,max)=>typeof v==='string'&&/^[1-9][0-9]*$/.test(v)&&(v.length<max.length||v.length===max.length&&v<=max);
      const id=v=>Number.isInteger(v)&&v>0&&v<=0xffffffff;
      const now=Date.now();
      if(!exactDataFields(value,['window_handle','process_id','thread_id','process_created_file_time','expires_at_unix_ms'])
          ||!decimal(value.window_handle,'9223372036854775807')||!decimal(value.process_created_file_time,'18446744073709551615')
          ||!id(value.process_id)||!id(value.thread_id)||!Number.isSafeInteger(value.expires_at_unix_ms)
          ||value.expires_at_unix_ms<=now||value.expires_at_unix_ms-now>5000)
        throw raiFault(method,'invalid_response','not_started');
      const snapshot=Object.freeze({...value});raiResidents.add(snapshot);return snapshot;
    });
  }
  function requestRaiShotRegion(args) {
    const method='raishot.resident.region',snapshot=args[0];
    if(args.length!==1||!raiResidents.has(snapshot))throw raiFault(method,'invalid_input','not_started');
    raiResidents.delete(snapshot);
    const now=Date.now();
    if(snapshot.expires_at_unix_ms<=now||snapshot.expires_at_unix_ms-now>5000)
      throw raiFault(method,'raishot_resident_expired','not_started');
    return raiCall(method,{resident:{...snapshot}},value=>{
      if(!exactDataFields(value,['queued','screenshot_completed'])||value.queued!==true||value.screenshot_completed!==false)
        throw raiFault(method,'invalid_response','unknown');
      return Object.freeze({queued:true,screenshotCompleted:false});
    });
  }

  // 插入 createDesktopApi 函数体；同步 Script 与异步 Node 共用，不创建进程。
  function createRaiShotResultApi(invoke) {
    const capture='raishot.capture_result',release='raishot.release_result';
    const fail=(method,code,effectState)=>{throw Object.assign(new Error('截图文件结果未获确认；不会自动重试。'),{method,code,effectState,retryAllowed:false});};
    function data(value,names) {
      if(!value||typeof value!=='object'||Array.isArray(value))throw new Error();
      const fields=Object.getOwnPropertyDescriptors(value);
      if(Reflect.ownKeys(fields).length!==names.length||names.some(k=>!Object.hasOwn(fields,k)||!Object.hasOwn(fields[k],'value')))throw new Error();
      return Object.fromEntries(names.map(k=>[k,fields[k].value]));
    }
    const id=x=>typeof x==='string'&&/^[0-9a-f]{32}$/.test(x);
    const bytes=text=>{let n=0;for(const c of text){const p=c.codePointAt(0);n+=p<128?1:p<2048?2:p<65536?3:4;}return n;};
    function result(value) {
      try {
        const v=data(value,['status','requestId','completionScope','image','errorCode','imageEffectState','artifactVerified',
          'automaticClipboardWritten','automaticHistoryAdded','sourceBusinessComplete','sourceAutoAdsorbVerified','retention','warningCode','retryAllowed']);
        if(!id(v.requestId)||v.completionScope!=='png_file_only'||v.automaticClipboardWritten!==false
          ||v.automaticHistoryAdded!==false||v.sourceBusinessComplete!==false||v.sourceAutoAdsorbVerified!==false
          ||v.retention!=='caller_owned_until_explicit_release'||v.retryAllowed!==false)throw new Error();
        if(v.status==='completed') {
          const image=data(v.image,['path','fileName','format','width','height','byteLength','sha256']);
          const path=typeof image.path==='string'?image.path.replaceAll('/','\\'):null;
          if(v.artifactVerified!==true||v.imageEffectState!=='confirmed'||v.errorCode!==null
            ||![null,'worker_cleanup_failed'].includes(v.warningCode)||image.fileName!=='image.png'||image.format!=='png'
            ||!Number.isInteger(image.width)||!Number.isInteger(image.height)||image.width<1||image.height<1
            ||image.width>65535||image.height>65535||image.width*image.height>32000000
            ||typeof image.byteLength!=='string'||!/^[1-9][0-9]{0,7}$/.test(image.byteLength)||Number(image.byteLength)>67108864
            ||typeof image.sha256!=='string'||!/^[0-9a-f]{64}$/.test(image.sha256)||path===null
            ||!path.endsWith('\\capture-'+v.requestId+'\\image.png')||!/^[A-Za-z]:[\\]/.test(path)
            ||bytes(path)>2048||/[\u0000-\u001f\u007f-\u009f<>:"|?*]/.test(path.slice(2))||/[\ud800-\udfff]/u.test(path)
            ||path.slice(3).split('\\').some(x=>!x||x==='.'||x==='..'||/[. ]$/.test(x)||/^(onedrive|icloud)/i.test(x)||/^dropbox$/i.test(x)))throw new Error();
          v.image=Object.freeze(image);
        } else if(!['cancelled','failed'].includes(v.status)||v.artifactVerified!==false||v.image!==null
          ||!['not_started','unknown'].includes(v.imageEffectState)||typeof v.errorCode!=='string'||!/^[a-z0-9_]{1,64}$/.test(v.errorCode)
          ||v.warningCode!==null)throw new Error();
        return Object.freeze(v);
      } catch(_){fail(capture,'invalid_response','unknown');}
    }
    function call(method,params,checked) {
      const failed=cause=>{
        const code=typeof cause?.code==='string'&&/^[a-z0-9_]{1,64}$/.test(cause.code)?cause.code:'native_bridge_failed';
        return fail(method,code,cause?.effectState==='not_started'?'not_started':'unknown');
      };
      try {
        const value=invoke(method,params);
        return value&&typeof value.then==='function'?value.then(checked).catch(failed):checked(value);
      } catch(cause){return failed(cause);}
    }
    return Object.freeze({
      captureRaiShotResult(...args) {
        let options;
        try {
          if(args.length>1)throw new Error();
          options=args.length===0||args[0]===undefined?{timeoutMs:30000}:data(args[0],['timeoutMs']);
          if(!Number.isInteger(options.timeoutMs)||options.timeoutMs<1||options.timeoutMs>30000)throw new Error();
        } catch(_){fail(capture,'invalid_input','not_started');}
        return call(capture,{timeout_ms:options.timeoutMs},result);
      },
      releaseRaiShotResult(...args) {
        if(args.length!==1||!id(args[0]))fail(release,'invalid_input','not_started');
        const requestId=args[0];
        return call(release,{request_id:requestId},value=>{
          try {
            const v=data(value,['released','requestId','scope','sessionDirectoryRetained']);
            if(v.released!==true||v.requestId!==requestId||v.scope!=='owned_png_file'||v.sessionDirectoryRetained!==true)throw new Error();
            return Object.freeze(v);
          } catch(_){fail(release,'invalid_response','unknown');}
        });
      }
    });
  }

  function ownedLeftCall(method,args) {
    if(args.length!==0)invalid(method,'左键调用不接受参数；owner和原目标由当前Service提供。');
    const failed=error=>{
      const codes=['invalid_input','invalid_response','input_owner_unavailable','input_cleanup_failed','input_busy','input_not_owned','input_interrupted','target_changed','target_unavailable','actions_unavailable','action_inactive','cancelled','timeout','input_failed','native_bridge_unavailable','desktop_unavailable','desktop_protocol'];
      let code='input_failed',effectState='unknown';
      try {
        const reportedCode=Object.getOwnPropertyDescriptor(error,'code')?.value;
        const reportedEffect=Object.getOwnPropertyDescriptor(error,'effectState')?.value;
        if(codes.includes(reportedCode))code=reportedCode;
        if(['unknown','not_started'].includes(reportedEffect))effectState=reportedEffect;
      }catch(_){}
      throw Object.assign(new Error('自有左键输入未获完整确认；请检查错误代码与效果状态。'),{code,method,effectState:code==='input_cleanup_failed'?'unknown':effectState});
    };
    const checked=value=>{
      try {
        const field=method==='mouse.left_down'?'submitted':'ownedPressEnded';
        const result=geometryFields(value,[field,'effectState']);
        if(result[field]!==true||result.effectState!=='unknown')throw new Error();
        return Object.freeze(result);
      } catch(_) {throw Object.assign(new Error('左键输入回执无效。'),{code:'invalid_response',method,effectState:'unknown'});}
    };
    try {const value=invoke(method,{});return value&&typeof value.then==='function'?value.then(checked,failed):checked(value);}
    catch(error){return failed(error);}
  }
  const desktop = Object.freeze({
    pressLeftButton: (...args)=>ownedLeftCall("mouse.left_down",args),
    releaseLeftButton: (...args)=>ownedLeftCall("mouse.left_up",args),
    ...createRaiShotResultApi(invoke),
    findRaiShotResident: (...args) => findRaiShotResident(args),
    requestRaiShotRegion: (...args) => requestRaiShotRegion(args),
    win32EditReplaceSelection: (...args) => win32EditReplaceSelection(args),
    explorerContext: (...args) => explorerContext(args),
    readSelectionText: (...args) => readSelectionText(args),
    getPointingRoot: (...args) => pointingRoot(args),
    handoffPointingRoot: (...args) => handoffPointingRoot(args),
    getWindowGeometryFor: (...args) => capturedWindow(args,1,'window.geometry',()=>({}),value=>geometryResult('window.geometry',value)),
    setWindowRectFor: (...args) => capturedWindow(args,3,'window.set_rect',()=>windowRectParams([args[1],args[2]]),value=>geometryResult('window.set_rect',value)),
    restoreWindowFor: (...args) => capturedWindow(args,1,'window.action',()=>({operation:'restore'}),value=>{
      if(!scopedRecord(value,['submitted'])||value.submitted!==true) throw Object.assign(new Error('还原回执无效。'),{code:'invalid_response',effectState:'unknown'});
      return {submitted:true};
    }),
    toggleMaximizeWindowFor: (...args) => capturedWindow(args,1,'window.action',()=>({operation:'toggle_maximize'}),value=>{
      if(!scopedRecord(value,['submitted'])||value.submitted!==true) throw Object.assign(new Error('最大化切换回执无效。'),{code:'invalid_response',effectState:'unknown'});
      return {submitted:true};
    }),
    trackWindowDragFor: (...args) => capturedWindow(args,2,'window.drag',()=>windowDragParams([args[1]]),value=>trackingResult('window.drag',value)),
    captureForegroundTarget,
    sendShortcutTo: (target, keys, options = {}) => {
      if (options === null || typeof options !== 'object' || Array.isArray(options)
          || Reflect.ownKeys(options).some(key => key !== 'keyDownDelayMs')) {
        throw scopedFault('target.keyboard.shortcut', 'invalid_input', 'not_started');
      }
      return scopedInput('target.keyboard.shortcut', target, desktop => desktop.sendShortcut(keys, options));
    },
    sendTextTo: (target, text) => scopedInput('target.keyboard.text', target, desktop => desktop.sendText(text)),
    adjustVolume: (...args) => adjustVolume(args),
    stepBrightness: (...args) => brightnessStep(args),
    target: () => invoke('target.info', {}),
    targetContext: () => invoke('target.context', {}),
    sendShortcut: (keys, options = {}) => {
      const method = 'keyboard.shortcut';
      if (!Array.isArray(keys) || keys.length < 1 || keys.length > 5
          || keys.some(key => !keyboardVirtualKey(key))) {
        invalid(method, '快捷键需要 1–5 个明确的键盘 VK 整数；修饰键在前，不接受鼠标按钮 VK。');
      }
      if (options?.timing !== undefined) {
        const timing = options.timing;
        const overrides = timing?.process_overrides ?? {};
        if (typeof options !== 'object' || Array.isArray(options) || Object.keys(options).length !== 1 || !timing || typeof timing !== 'object' || Array.isArray(timing)
            || Object.keys(timing).some(key => !['key_down_delay_ms','process_overrides'].includes(key))
            || (timing.key_down_delay_ms !== undefined && (!Number.isInteger(timing.key_down_delay_ms) || timing.key_down_delay_ms < 0 || timing.key_down_delay_ms > 1000))
            || !overrides || typeof overrides !== 'object' || Array.isArray(overrides) || Object.keys(overrides).length > 16
            || Object.values(overrides).some(value => !Number.isInteger(value) || value < 0 || value > 1000)
            || timing.process_overrides === null) {
          invalid(method, 'timing 需要原生快捷键时序对象，与 keyDownDelayMs 互斥。');
        }
        return invoke(method, {keys, timing});
      }
      const delay = keyTiming(options, 1, method);
      return invoke(method, delay ? {keys, key_down_delay_ms:delay} : {keys});
    },
    sendKeySequence: (modifiers, keys, options = {}) => {
      const method = 'keyboard.sequence';
      if (!Array.isArray(modifiers) || modifiers.length > 4 || !Array.isArray(keys) || keys.length < 1 || keys.length > 16
          || [...modifiers,...keys].some(key => !keyboardVirtualKey(key))) {
        invalid(method, '连续按键需要 0–4 个修饰键和 1–16 个普通键，不接受鼠标按钮 VK。');
      }
      return invoke(method, {modifiers, keys, key_down_delay_ms:keyTiming(options, keys.length, method)});
    },
    sendText: text => textCall('keyboard.text', text),
    copySelectionText: () => invoke('selection.copy', {}),
    copySelectionData: (...args) => {
      const method = 'selection.copy_data';
      let params;
      try {
        const options = args[0] === undefined ? {} : args[0];
        if (args.length > 1 || options === null || typeof options !== 'object' || Array.isArray(options)
            || Reflect.ownKeys(options).some(key => !['timeoutMs','pollIntervalMs','settleMs','allowUnchanged','format'].includes(key))) throw new Error();
        const {timeoutMs = 600, pollIntervalMs = 15, settleMs = 30, allowUnchanged = false, format = 'auto'} = options;
        if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 5000
            || !Number.isInteger(pollIntervalMs) || pollIntervalMs < 1 || pollIntervalMs > 1000 || pollIntervalMs > timeoutMs
            || !Number.isInteger(settleMs) || settleMs < 0 || settleMs > 1000 || typeof allowUnchanged !== 'boolean'
            || !['auto','text','files'].includes(format)) throw new Error();
        params = {timeout_ms:timeoutMs,poll_interval_ms:pollIntervalMs,settle_ms:settleMs,allow_unchanged:allowUnchanged,
          ...(format === 'auto' ? {} : {format})};
      } catch (_) { invalid(method, '复制选区数据只接受有效的 timeoutMs、pollIntervalMs、settleMs、allowUnchanged 和 format。'); }
      return dataCall(method, params);
    },
    getPointerState: (...args) => {
      if (args.length!==0) invalid('pointer.state','指针采样不接收参数。');
      return trackingCall('pointer.state',{});
    },
    trackWindowDrag: (...args) => trackingCall('window.drag',windowDragParams(args)),
    getWindowGeometry: (...args) => {
      if (args.length !== 0) invalid('window.geometry','窗口矩形读取不接收参数。');
      return geometryCall('window.geometry',{});
    },
    setWindowRect: (...args) => geometryCall('window.set_rect',windowRectParams(args)),
    windowAction: operation => {
      if (!['toggle_maximize', 'restore', 'toggle_topmost', 'minimize', 'close'].includes(operation)) {
        invalid('window.action', '窗口操作需要 toggle_maximize、restore、toggle_topmost、minimize 或 close。');
      }
      return invoke('window.action', {operation});
    },
  });
  const clipboard = Object.freeze({
    readText: () => invoke('clipboard.read', {}),
    readData: (...args) => {
      if (args.length !== 0) invalid('clipboard.read_data', 'readData 不接收参数。');
      return dataCall('clipboard.read_data', {});
    },
    writeFiles: (...args) => {
      const method = 'clipboard.write_files';
      let paths;
      try {
        if (args.length !== 1 || !Array.isArray(args[0])) throw new Error();
        const descriptors = Object.getOwnPropertyDescriptors(args[0]);
        const length = descriptors.length?.value;
        if (!Number.isInteger(length) || length < 1 || length > 64 || Reflect.ownKeys(descriptors).length !== length + 1) throw new Error();
        paths = []; let units = 0;
        for (let i = 0; i < length; i++) {
          const descriptor = descriptors[i];
          if (!descriptor || !Object.hasOwn(descriptor, 'value')) throw new Error();
          const path = descriptor.value;
          if (!dataText(path, 32767, false) || /[\u0000-\u001f<>"|?*]/.test(path)) throw new Error();
          let tail;
          if (/^[A-Za-z]:[\\/]/.test(path)) tail = path.slice(3);
          else if (path.startsWith('\\\\') && !path.startsWith('\\\\.\\')) {
            const pieces = path.slice(2).split(/[\\/]/);
            if (!pieces[0] || !pieces[1]) throw new Error();
            tail = path.slice(2);
          } else throw new Error();
          if (tail.includes(':') || tail.split(/[\\/]/).some(part => part === '.' || part === '..')) throw new Error();
          units += path.length; if (units > 65536) throw new Error();
          paths.push(path);
        }
      } catch (_) { invalid(method, '文件列表需要1–64项有效Unicode绝对Windows路径，每项最多32767、总计最多65536个UTF-16单元。'); }
      const failure = error => {
        let code = 'native_bridge_failed', effectState = 'unknown';
        try {
          if (typeof error?.code === 'string' && /^[A-Za-z0-9_]{1,64}$/.test(error.code)) code = error.code;
          if (error?.effectState === 'not_started') effectState = 'not_started';
        } catch (_) { /* 不回显异常getter或路径。 */ }
        throw Object.assign(new Error('文件列表剪贴板写入未获完整确认；请检查错误代码和效果状态。'), {code, effectState, method});
      };
      const receipt = value => {
        try {
          if (!exactDataFields(value, ['written','sequenceNumber'])) throw new Error();
          const written=value.written, sequenceNumber=value.sequenceNumber;
          if (written !== true || !Number.isInteger(sequenceNumber) || sequenceNumber < 0 || sequenceNumber > 0xffffffff) throw new Error();
          return Object.freeze({written:true,sequenceNumber});
        } catch (_) { throw Object.assign(new Error('文件列表剪贴板写入回执无效；不会自动重试。'), {code:'invalid_response',effectState:'unknown',method}); }
      };
      let result;
      try { result = invoke(method, {paths}); } catch (error) { return failure(error); }
      return result && typeof result.then === 'function' ? result.then(receipt, failure) : receipt(result);
    },

    writeText: (text, options = {}) => {
      if (options === null || typeof options !== 'object' || Array.isArray(options)
          || Object.keys(options).some(key => key !== 'excludeFromHistory')
          || ('excludeFromHistory' in options && typeof options.excludeFromHistory !== 'boolean')) {
        invalid('clipboard.write', '剪贴板选项只接受布尔值 excludeFromHistory。');
      }
      return textCall('clipboard.write', text, options.excludeFromHistory ? {exclude_from_history:true} : {});
    },
  });
  const ui = Object.freeze({
    setActionHint: options => {
      const method = 'ui.action_hint';
      const descriptor = options && typeof options === 'object' && !Array.isArray(options)
        ? Object.getOwnPropertyDescriptor(options, 'text') : undefined;
      const text = descriptor && Object.hasOwn(descriptor, 'value') ? descriptor.value : undefined;
      const hasPlacement = options && typeof options === 'object' && Object.hasOwn(options, 'placement');
      const placeDescriptor = hasPlacement ? Object.getOwnPropertyDescriptor(options, 'placement') : undefined;
      const placement = hasPlacement ? placeDescriptor && Object.hasOwn(placeDescriptor, 'value') ? placeDescriptor.value : undefined : 'inline';
      let valid = typeof text === 'string' && text.length > 0 && text.length <= 16 && text.trim() === text
        && !/[\x00-\x1f\x7f-\x9f\u2028\u2029\ufeff]/.test(text);
      if (valid) for (let i = 0; i < text.length; i++) {
        const unit = text.charCodeAt(i);
        if (unit >= 0xd800 && unit <= 0xdbff) {
          const next = text.charCodeAt(++i);
          if (!(next >= 0xdc00 && next <= 0xdfff)) valid = false;
        } else if (unit >= 0xdc00 && unit <= 0xdfff) valid = false;
      }
      if (!valid || Reflect.ownKeys(options).some(key => !['text','placement'].includes(key))
          || !['inline','topRight'].includes(placement)) invalid(method, '动作版本提示只接受有效text与inline/topRight位置。');
      const receipt = value => {
        if (!value || typeof value !== 'object' || Array.isArray(value)
            || Reflect.ownKeys(value).length !== 1 || value.updated !== true) {
          throw Object.assign(new Error('动作版本提示更新未获确认，不会自动重试。'), {code:'invalid_response', method, effectState:'unknown'});
        }
        return {updated:true};
      };
      const result = invoke(method, {text, ...(hasPlacement ? {placement} : {})});
      return result && typeof result.then === 'function' ? result.then(receipt) : receipt(result);
    },
    toggleHistory: (...args) => {
      const method = 'ui.toggle_history';
      if (args.length !== 0) invalid(method, '切换历史窗口不接受参数。');
      const receipt = value => {
        const keys = value && typeof value === 'object' && !Array.isArray(value) ? Reflect.ownKeys(value) : [];
        if (keys.length === 1 && keys[0] === 'opened' && value.opened === true) return {opened:true};
        if (keys.length === 1 && keys[0] === 'closed' && value.closed === true) return {closed:true};
        throw Object.assign(new Error('历史窗口切换回执未确认，不会自动重试。'),
          {code:'invalid_response', method, effectState:'unknown'});
      };
      const result = invoke(method, {});
      return result && typeof result.then === 'function' ? result.then(receipt) : receipt(result);
    },
    openHistory: (...args) => {
      const method = 'ui.open_history';
      if (args.length !== 0) invalid(method, '打开历史窗口不接受参数。');
      const receipt = value => {
        if (!value || typeof value !== 'object' || Array.isArray(value)
            || Reflect.ownKeys(value).length !== 1 || value.opened !== true) {
          throw Object.assign(new Error('历史窗口创建回执未确认，不会自动重试。'),
            {code:'invalid_response', method, effectState:'unknown'});
        }
        return {opened:true};
      };
      const result = invoke(method, {});
      return result && typeof result.then === 'function' ? result.then(receipt) : receipt(result);
    },
    notify: options => {
      const method = 'ui.notify';
      const text = (value, max, multiline) => {
        // 与原生 Rust 校验共用 Unicode White_Space 定义；FEFF 是格式字符，不是空白。
        if (typeof value !== 'string' || !/[^\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/.test(value) || value.length > max
            || (multiline ? /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/ : /[\x00-\x1f\x7f-\x9f\u2028\u2029]/).test(value)) return false;
        for (let i = 0; i < value.length; i++) {
          const unit = value.charCodeAt(i);
          if (unit >= 0xd800 && unit <= 0xdbff) {
            const next = value.charCodeAt(++i);
            if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
          } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
        }
        return true;
      };
      if (!options || typeof options !== 'object' || Array.isArray(options)
          || Reflect.ownKeys(options).some(key => !['message','title','level','placement','durationMs'].includes(key))) {
        invalid(method, '通知参数只接受 message、title、level、placement 和 durationMs。');
      }
      const {message, title = 'OmniTrigger', level = 'info', placement = 'topRight', durationMs = 3000} = options;
      if (!text(message, 1024, true) || !text(title, 128, false)
          || !['info','success','warning','error'].includes(level)
          || !['topRight','bottomRight','center'].includes(placement)
          || !Number.isInteger(durationMs) || durationMs < 1000 || durationMs > 30000) {
        invalid(method, '通知需要 1–1024 个 UTF-16 单元的有效正文、最多 128 单元的单行标题、有效级别和位置，以及 1000–30000 毫秒的整数时长。');
      }
      // 两类宿主共享同一份纯文本参数；不裁剪、不转义或更改作者正文。
      return invoke(method, {message, title, level, placement, durationMs});
    },
    choose: options => {
      const method = 'ui.choose';
      const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
      const text = (value, max) => {
        if (typeof value !== 'string' || !value.trim() || value.length > max || /[\x00-\x1f\x7f-\x9f\u2028\u2029]/.test(value)) return false;
        for (let i = 0; i < value.length; i++) {
          const unit = value.charCodeAt(i);
          if (unit >= 0xd800 && unit <= 0xdbff) {
            const next = value.charCodeAt(++i);
            if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
          } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
        }
        return true;
      };
      const ids = new Set();
      if (!object(options) || Object.keys(options).some(key => !['items','title','placeholder','timeoutMs'].includes(key))
          || !Array.isArray(options.items) || options.items.length < 1 || options.items.length > 200
          || (options.title !== undefined && !text(options.title, 128))
          || (options.placeholder !== undefined && !text(options.placeholder, 128))
          || (options.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > 60000))
          || options.items.some(item => {
            if (!object(item) || Object.keys(item).some(key => !['id','label','description'].includes(key))
                || !text(item.id, 128) || !text(item.label, 256)
                || (item.description !== undefined && item.description !== '' && !text(item.description, 512)) || ids.has(item.id)) return true;
            ids.add(item.id);
            return false;
          })) invalid(method, '选择需要 1–200 个唯一 ID 的候选项和有效单行文本，时限 1–60000 毫秒。');
      // 复制白名单字段；两种引擎传递同一份参数，不传递业务对象或回调。
      const params = {items:options.items.map(item => ({id:item.id,label:item.label,...(item.description === undefined ? {} : {description:item.description})})),
        ...(options.title === undefined ? {} : {title:options.title}),
        ...(options.placeholder === undefined ? {} : {placeholder:options.placeholder}),
        ...(options.timeoutMs === undefined ? {} : {timeoutMs:options.timeoutMs})};
      let bytes = 0;
      for (const character of JSON.stringify(params)) {
        const point = character.codePointAt(0);
        bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
      }
      if (bytes > 32*1024) invalid(method, '选择参数总量最多 32 KiB。');
      return invoke(method, params);
    },
  });
  function fsFault(method, code = 'invalid_input') {
    throw Object.assign(new Error(method === 'fs.sort_logical_paths' ? '字符串逻辑排序失败；请检查错误代码。' : '本地文件查询失败；请检查错误代码。'), {code, effectState:'not_started', method});
  }
  function fsPath(path, fail) {
    if (typeof path !== 'string' || path.length < 3 || path.length > 32767
        || !/^[A-Za-z]:[\\/]/.test(path) || /[\x00-\x1f\x7f-\x9f]/.test(path)) fail();
    for (let i = 0; i < path.length; i++) {
      const unit = path.charCodeAt(i);
      if (unit >= 0xd800 && unit <= 0xdbff) {
        const next = path.charCodeAt(++i);
        if (!(next >= 0xdc00 && next <= 0xdfff)) fail();
      } else if (unit >= 0xdc00 && unit <= 0xdfff) fail();
    }
    // 仅对检查副本统一分隔符，查询仍传原文。字符串 split 避免 Boa 正则 split
    // 在最大长度路径上逐位置尝试匹配。
    const components = path.length === 3 ? [] : path.slice(3).replaceAll(String.fromCharCode(92), '/').split('/');
    if (components.at(-1) === '') components.pop();
    for (const component of components) {
      if (!component || component === '.' || component === '..' || /[<>:"|?*]/.test(component) || /[ .]$/.test(component)
          || /^(?:CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])(?: *\.| *$)/i.test(component)) fail();
    }
    if (components.some(component => {
      const name = component.toLowerCase();
      return name.startsWith('onedrive') || name.startsWith('icloud') || name === 'dropbox';
    })) fail('path_unavailable');
  }
  function fsCall(method, params, result) {
    const failure = error => {
      let code = 'fs_error';
      try {
        const reported = error?.code;
        const codes = ['invalid_input','path_not_local','path_unavailable','access_denied','fs_error','fs_unavailable','fs_busy','cancelled','timeout'];
        if (method === 'fs.read_directory') codes.push('path_missing','not_directory','directory_too_large','directory_changed');
        if (method === 'fs.sort_logical_paths') codes.push('desktop_call_limit');
        if (codes.includes(reported)) code = reported;
      } catch (_) { /* 不读取或回显异常 getter 的正文。 */ }
      fsFault(method, code);
    };
    try {
      const value = invoke(method, params);
      if (method === 'fs.sort_logical_paths') {
        // 先校验数组，不为识别Promise读取坏回执上的then getter。
        if (Array.isArray(value)) return result(value);
        try { return Promise.prototype.then.call(value, result, failure); }
        catch (_) { fsFault(method, 'fs_error'); }
      }
      return value && typeof value.then === 'function' ? value.then(result, failure) : result(value);
    } catch (error) { return failure(error); }
  }
  function directoryMetadata(value, count, ordinals) {
    const fields = ['ordinal','attributes','creationTimeUtc100ns','lastAccessTimeUtc100ns','lastWriteTimeUtc100ns',
      'creationTimeLocalTicks100ns','lastAccessTimeLocalTicks100ns','lastWriteTimeLocalTicks100ns','timeZoneKey'];
    const fail = () => fsFault('fs.read_directory', 'fs_error');
    if (!exactDataFields(value, fields) || !Number.isInteger(value.ordinal) || value.ordinal < 0 || value.ordinal >= count
        || ordinals.has(value.ordinal) || !Number.isInteger(value.attributes) || value.attributes < 0 || value.attributes > 0xffffffff
        || !dataText(value.timeZoneKey, 128, true)) fail();
    const decimal = (text, maximum) => typeof text === 'string' && /^(0|[1-9][0-9]{0,18})$/.test(text)
      && (text.length < maximum.length || text.length === maximum.length && text <= maximum);
    for (const field of fields.slice(2,5)) if (!decimal(value[field], '2650467743999999999')) fail();
    for (const field of fields.slice(5,8)) if (!decimal(value[field], '3155378975999999999')) fail();
    ordinals.add(value.ordinal);
    return Object.freeze(Object.fromEntries(fields.map(field => [field,value[field]])));
  }

  function directoryResult(value, path, maxEntries, withMetadata = false) {
    const fail = () => fsFault('fs.read_directory', 'fs_error');
    try {
      if (!exactDataFields(value, ['entries','complete']) || value.complete !== true
          || !Array.isArray(value.entries) || value.entries.length > maxEntries
          || Reflect.ownKeys(value.entries).length !== value.entries.length + 1) fail();
      const entries = [], names = new Set(), ordinals = new Set();
      for (let i = 0; i < value.entries.length; i++) {
        if (!Object.hasOwn(value.entries, i)) fail();
        const entry = value.entries[i];
        if (!exactDataFields(entry, withMetadata ? ['name','path','kind','reason','metadata'] : ['name','path','kind','reason'])) fail();
        const {name, path:childPath, kind, reason} = entry;
        if (!dataText(name, 32767, false) || name === '.' || name === '..' || /[\\/]/.test(name)
            || !dataText(childPath, 32767, false) || names.has(name)
            || childPath !== path + (/[\\/]$/.test(path) ? '' : String.fromCharCode(92)) + name
            || !['file','directory','other','unavailable'].includes(kind)
            || (kind === 'unavailable' ? !['reparse_point','cloud_placeholder','cloud_path','unsupported_name'].includes(reason) : reason !== null)) fail();
        if (kind !== 'unavailable') fsPath(childPath, fail);
        names.add(name);
        const resultEntry={name,path:childPath,kind,reason};
        if (withMetadata) resultEntry.metadata=directoryMetadata(entry.metadata,value.entries.length,ordinals);
        entries.push(Object.freeze(resultEntry));
      }
      const result = {entries, complete:true};
      // 对清理后的 DTO 计 UTF-8 JSON 字节，既不依赖 Node Buffer，也不丢失代理对。
      let bytes = 0;
      for (const character of JSON.stringify(result)) {
        const point = character.codePointAt(0);
        bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
        if (bytes > 64*1024) fail();
      }
      Object.freeze(entries);
      return Object.freeze(result);
    } catch (_) { fail(); }
  }
  // 仅处理调用方给定字符串，不套用fsPath，不读取文件或选择目标。
  function logicalPaths(value, fail, budget = false) {
    try {
      if (!Array.isArray(value) || Object.getPrototypeOf(value) !== Array.prototype) fail();
      const keys = Reflect.ownKeys(value);
      const length = Object.getOwnPropertyDescriptor(value, 'length')?.value;
      if (!Number.isInteger(length) || length < 0 || length > 1024 || keys.length !== length + 1) fail();
      const paths = [];
      let bytes = 12; // {"paths":[]}，逐项加字符串JSON与逗号，禁止分配完整大JSON。
      for (let index = 0; index < length; index++) {
        const descriptor = Object.getOwnPropertyDescriptor(value, String(index));
        if (!descriptor || !Object.hasOwn(descriptor, 'value')) fail();
        const text = descriptor.value;
        if (typeof text !== 'string' || text.length > 32767) fail();
        for (let unit = 0; unit < text.length; unit++) {
          const code = text.charCodeAt(unit);
          if (code >= 0xd800 && code <= 0xdbff) {
            const next = text.charCodeAt(++unit);
            if (!(next >= 0xdc00 && next <= 0xdfff)) fail();
          } else if (code >= 0xdc00 && code <= 0xdfff) fail();
        }
        if (budget) {
          if (index > 0) bytes++;
          for (const character of JSON.stringify(text)) {
            const point = character.codePointAt(0);
            bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
            if (bytes > 64*1024) fail();
          }
        }
        paths.push(text);
      }
      return paths;
    } catch (_) { fail(); }
  }
  const fs = Object.freeze({
    stat: (...args) => {
      const method = 'fs.stat';
      const fail = code => fsFault(method, code);
      if (args.length !== 1) fail();
      const path = args[0];
      fsPath(path, fail);
      // 平台独立复验盘类型、物理卷、祖先属性和占位文件。
      return fsCall(method, {path}, value => {
        if (!exactDataFields(value, ['kind']) || !['file','directory','missing','other'].includes(value.kind)) fail('fs_error');
        return value;
      });
    },
    sortLogicalPaths: (...args) => {
      const method = 'fs.sort_logical_paths';
      const fail = code => fsFault(method, code);
      if (args.length !== 1) fail();
      const paths = Object.freeze(logicalPaths(args[0], fail, true));
      const counts = new Map();
      for (const path of paths) counts.set(path, (counts.get(path) ?? 0) + 1);
      return fsCall(method, {paths}, value => {
        const invalid = () => fail('fs_error');
        const result = logicalPaths(value, invalid);
        if (result.length !== paths.length) invalid();
        const remaining = new Map(counts);
        for (const path of result) {
          const count = remaining.get(path) ?? 0;
          if (count === 0) invalid();
          remaining.set(path, count - 1);
        }
        return Object.freeze(result);
      });
    },
    readDirectory: (...args) => {
      const method = 'fs.read_directory';
      const fail = code => fsFault(method, code);
      if (args.length < 1 || args.length > 2) fail();
      const [path, supplied] = args;
      fsPath(path, fail);
      let maxEntries, order, metadata, entryType, searchPattern, matchMode;
      try {
        const options = supplied === undefined ? {} : supplied;
        if (!options || typeof options !== 'object' || Array.isArray(options)
            || Reflect.ownKeys(options).some(key => !['maxEntries','order','metadata','entryType','searchPattern','matchMode'].includes(key))) fail();
        const own = key => Object.hasOwn(options, key) ? options[key] : undefined;
        const suppliedMax = own('maxEntries');
        maxEntries = suppliedMax === undefined ? 128 : suppliedMax;
        order = own('order'); metadata = own('metadata'); entryType = own('entryType');
        searchPattern = own('searchPattern'); matchMode = own('matchMode');
        if (!Number.isInteger(maxEntries) || maxEntries < 1 || maxEntries > 1024
            || (order !== undefined && !['name','native'].includes(order))
            || (metadata !== undefined && typeof metadata !== 'boolean')
            || (entryType !== undefined && !['all','file','directory'].includes(entryType))
            || (searchPattern === undefined ? matchMode !== undefined : matchMode !== 'windows-find')
            || (searchPattern !== undefined && (!dataText(searchPattern,256,false)
              || /[\u0000-\u001f\u007f-\u009f\\/:<>"|]/.test(searchPattern)
              || !searchPattern.replace(/ +$/, '') || searchPattern.replace(/ +$/, '').endsWith('..')))) fail();
      } catch (_) { fail(); }
      const params = {path, max_entries:maxEntries};
      if (order !== undefined) params.order = order;
      if (metadata !== undefined) params.metadata = metadata;
      if (entryType !== undefined) params.entry_type = entryType;
      if (searchPattern !== undefined) { params.search_pattern = searchPattern; params.match_mode = matchMode; }
      return fsCall(method, params, value => directoryResult(value, path, maxEntries, metadata === true));
    },
  });
  return {desktop, clipboard, ui, fs};
}
