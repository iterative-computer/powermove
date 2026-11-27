import { replayScenario, scenarios } from "../tests/replay";
let failed = false;
for (const scenario of scenarios) {
  const result = await replayScenario(scenario);
  console.log(
    `${scenario}: ${result.matched}/${result.boundaries} public boundaries; ${result.differences.length} differences (${result.scope})`,
  );
  for (const difference of result.differences) console.error(difference);
  failed ||= result.differences.length !== 0;
}
if (failed) process.exitCode = 1;
