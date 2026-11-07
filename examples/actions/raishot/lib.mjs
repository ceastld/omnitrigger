// .NET Char.IsWhiteSpace：包含 U+0085，不包含 U+FEFF。
const modeWhitespace=new Set([9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288]);
/** @param {string} value */
function trimMode(value) {
  let start=0,end=value.length;
  while(start<end && modeWhitespace.has(value.charCodeAt(start))) start++;
  while(end>start && modeWhitespace.has(value.charCodeAt(end-1))) end--;
  return value.slice(start,end);
}
const modes = new Set(['region', 'tray', 'verify']);
/** @param {string} code @param {string} message */
function fail(code, message) { return Object.assign(new Error(message), {code, effectState:'not_started', retryAllowed:false}); }
/** @param {unknown} value */
export function absoluteDirectory(value) {
  if (typeof value !== 'string' || value.length <= 3 || value.length > 2048 || !/^[A-Za-z]:[\\/]/.test(value) || /[\x00-\x1f<>"|?*]/.test(value) || value.slice(2).includes(':')) {
    throw fail('invalid_input', '依赖目录必须是本机绝对路径。');
  }
  const parts=value.slice(3).split(/[\\/]/);
  if (parts.some(p => p === '.' || p === '..' || /[. ]$/.test(p) || /^(?:OneDrive[^\\/]*|Dropbox|iCloud[^\\/]*)$/i.test(p))) {
    throw fail('invalid_input', '依赖目录包含不支持的路径段。');
  }
  return value.replace(/\//g,'\\').replace(/\\+$/,'');
}
/** @param {Record<string, any>} input */
export function configuration(input) {
  // 非空显式参数优先于 launch_mode；null 默认 region，空白模式拒绝。
  const explicit=input.quicker_in_param;
  const raw=typeof explicit==='string' && trimMode(explicit)!=='' ? explicit : (input.launch_mode ?? 'region');
  if (typeof raw!=='string') throw fail('invalid_input','启动方式必须为 region、tray 或 verify。');
  const mode=trimMode(raw).toLowerCase();
  if (!modes.has(mode)) throw fail('invalid_input','启动方式必须为 region、tray 或 verify。');
  if (explicit!=null && typeof explicit!=='string') throw fail('invalid_input','启动参数必须为字符串。');
  const directory=input.packageDirectory == null ? '' : absoluteDirectory(input.packageDirectory);
  return {mode,directory};
}
/** @param {string|null} packageRoot @param {string} mode @param {string} directory @param {unknown} powerShellProgram */
export function prepareCall(packageRoot,mode,directory,powerShellProgram) {
  if (typeof powerShellProgram!=='string' || !/^[A-Za-z]:[\\/]/.test(powerShellProgram) || !/\.exe$/i.test(powerShellProgram)) {
    throw fail('invalid_input','需要显式开发输入 powerShellProgram：本机绝对 .exe 路径。');
  }
  const program=absoluteDirectory(powerShellProgram);
  const root=absoluteDirectory(packageRoot);
  const args=['-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',`${root}\\package-cli.ps1`,'-Operation',mode==='verify'?'verify':'prepare'];
  if(directory) args.push('-PackageDirectory',directory);
  // capture 只接受显式绝对 EXE；不搜索 PATH，也不提供隐含系统目录默认值。
  return {program,args,options:{timeoutMs:60000,allowNonZero:true,cwd:root}};
}
/** @param {{code:number,stdout:string,stderr:string}} result @param {string} mode */
export function packageResult(result,mode) {
  if(result.code!==0 || typeof result.stdout!=='string' || result.stdout.length>8192) throw fail('raishot_package_failed','RaiShot 依赖准备或校验失败。');
  let value;
  try {value=JSON.parse(result.stdout.replace(/^\uFEFF/,''));} catch {throw fail('raishot_package_failed','RaiShot 依赖返回无效状态。');}
  if(!value || typeof value!=='object' || Array.isArray(value)) throw fail('raishot_package_failed','RaiShot 依赖返回无效状态。');
  const directory=absoluteDirectory(value.directory);
  if(value.exe_path!==`${directory}\\raishot.exe` || typeof value.version!=='string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value.version) ||
     !['cea.rustshot','rustshot'].includes(value.package_name) || !['full','size'].includes(value.verification) || !['existing','downloaded'].includes(value.source) || typeof value.archive_authenticated!=='boolean') {
    throw fail('raishot_package_failed','RaiShot 依赖状态与启动要求不符。');
  }
  /** @type {number[]} */
  const parts=value.version.split('.').map(Number);
  if(parts.some(v=>!Number.isSafeInteger(v)) || (parts[0]===0 && (parts[1]<4 || (parts[1]===4 && parts[2]<18))) || (mode==='verify' && value.verification!=='full')) {
    throw fail('raishot_package_failed','RaiShot 依赖版本过旧或未完成校验。');
  }
  return Object.freeze({...value,directory});
}
/** @param {{pid:number}} result @param {string} mode */
export function launched(result,mode) {
  if(!Number.isSafeInteger(result?.pid) || result.pid<=0) throw Object.assign(new Error('RaiShot 启动结果未确认。'),{code:'raishot_launch_unknown',effectState:'unknown',retryAllowed:false});
  return {status:'launched',mode,pid:result.pid,screenshotCompleted:false};
}
/** @param {{readonly aborted:boolean}} signal */
export function cancelled(signal) { if(signal.aborted) throw fail('cancelled','RaiShot 调用已取消。'); }
