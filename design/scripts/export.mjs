// 用应用自身的代码导出预览数据：默认导出仓库夹具，传入参数时原样转交给应用，
// 例如 `npm run export -- --profile ui-browse data/local/ui-browse.json`。
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const design = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repo = path.dirname(design);
const extra = process.argv.slice(2);
const output = extra.length > 0 ? extra.pop() : path.join(design, "data", "ui-browse.json");
const source =
  extra.length > 0
    ? extra
    : ["--config", path.join(repo, "tests", "fixtures", "profiles", "ui-browse", "bindings.json")];

const result = spawnSync(
  "cargo",
  [
    "run",
    "--quiet",
    "-p",
    "omnitrigger-app",
    "--",
    ...source,
    "--export-trigger-preview",
    path.resolve(design, output),
  ],
  { cwd: repo, stdio: "inherit", shell: process.platform === "win32" },
);
process.exit(result.status ?? 1);
