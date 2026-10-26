import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
const program = process.env.RAISHOT_CLI || 'raishot-cli.exe';

await execFileAsync(program, ['--capture'], { windowsHide: true });
console.log('RaiShot 截图进程正常退出。');
