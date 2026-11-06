import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath, pathToFileURL} from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const parent = join(repo, '.tmp', 'project-write');
mkdirSync(parent, {recursive: true});

function fixture() {
  const root = mkdtempSync(join(parent, 'case-'));
  const project = join(root, 'project');
  mkdirSync(join(project, 'sdk'), {recursive: true});
  writeFileSync(join(project, 'action.json'), JSON.stringify({schema_version: 1, actions: [{runtime: 'node'}]}));
  writeFileSync(join(project, 'main.mjs'), 'throw Error("AUTHOR CODE MUST NOT RUN");\n');
  writeFileSync(join(project, 'sdk', 'api.d.ts'), 'old API\n');
  writeFileSync(join(project, 'sdk', 'clipboard-history-api.d.ts'), 'old history API\n');
  return {root, project};
}

function refresh(project, flags = []) {
  return spawnSync(process.execPath, [...flags, join(repo, 'tools/script-project.mjs'),
    'refresh', '--runtime', 'node', '--project', project], {
    encoding: 'utf8', windowsHide: true, timeout: 60000,
  });
}

test('实际刷新只替换SDK声明并保留作者文件', () => {
  const {project} = fixture();
  const before = ['action.json', 'main.mjs'].map(name => readFileSync(join(project, name)));
  const result = refresh(project);
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const receipt = JSON.parse(result.stdout);
  assert.equal(receipt.status, 'refreshed');
  assert.equal(receipt.author_files_changed, false);
  assert.equal(receipt.author_code_executed, false);
  for (const [index, name] of ['action.json', 'main.mjs'].entries()) {
    assert.deepEqual(readFileSync(join(project, name)), before[index]);
  }
  for (const name of ['api.d.ts', 'clipboard-history-api.d.ts']) {
    assert.deepEqual(readFileSync(join(project, 'sdk', name)),
      readFileSync(join(repo, 'crates/omnitrigger-actions/runtime', name)));
  }
  assert(readdirSync(join(project, 'sdk')).every(name => !name.endsWith('.tmp')));
});

test('临时文件碰撞拒绝刷新且不删除已有文件', () => {
  const {root, project} = fixture();
  const preload = join(root, 'collision.mjs');
  writeFileSync(preload, `import crypto from 'node:crypto';
import {syncBuiltinESMExports} from 'node:module';
crypto.randomUUID = () => 'collision';
syncBuiltinESMExports();\n`);
  const temporary = join(project, 'sdk', 'api.d.ts.sdk-collision.tmp');
  writeFileSync(temporary, 'EXISTING FILE MUST SURVIVE\n');
  const result = refresh(project, ['--import', pathToFileURL(preload).href]);
  assert.ifError(result.error);
  assert.equal(result.status, 1, result.stdout);
  assert.match(result.stderr, /^\s*\{/, result.stderr);
  assert.equal(JSON.parse(result.stderr).error.code, 'project_failed');
  assert.equal(readFileSync(temporary, 'utf8'), 'EXISTING FILE MUST SURVIVE\n');
  assert.equal(readFileSync(join(project, 'sdk', 'api.d.ts'), 'utf8'), 'old API\n');
  assert.equal(readFileSync(join(project, 'sdk', 'clipboard-history-api.d.ts'), 'utf8'), 'old history API\n');
});
