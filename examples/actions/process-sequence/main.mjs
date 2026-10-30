// 两个步骤可以是 EXE，也可以显式指定解释器及脚本参数。没有隐式 shell。
// 示例输入：{"steps":[{"program":"node","args":["--version"]},{"program":"node","args":["-p","'第二步完成'"]}]}
export default async function (ctx) {
  const { steps } = ctx.input;
  if (!Array.isArray(steps) || steps.length !== 2) {
    throw Object.assign(new Error('请通过 steps 提供两个程序步骤。'), { code: 'invalid_input' });
  }
  const results = [];
  for (const step of steps) {
    if (!step || typeof step !== 'object') {
      throw Object.assign(new Error('每一步需要 program、args 和可选 cwd。'), { code: 'invalid_input' });
    }
    results.push(await ctx.process.run(step.program, step.args ?? [], { cwd: step.cwd }));
  }
  // 前一步失败会抛错，后一步不会执行。这里仅确认两个进程正常退出。
  return { completed: results.length, results };
}
