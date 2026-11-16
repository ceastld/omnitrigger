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
