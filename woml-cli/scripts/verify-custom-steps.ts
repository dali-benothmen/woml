import { cp, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

const projectRoot = resolve(import.meta.dir, '../..');
const fixtureRoot = resolve(
  projectRoot,
  'woml/tests/fixtures/reusable-definitions'
);
const temporaryRoot = await mkdtemp(resolve(tmpdir(), 'woml-custom-step-'));

try {
  for (const name of [
    'custom-step-workflow.woml',
    'calculate-discount.woml',
    'pricing.ts',
    'pricing-helper.ts',
  ]) {
    await cp(resolve(fixtureRoot, name), resolve(temporaryRoot, name));
  }
  const statePath = resolve(temporaryRoot, 'state.sqlite');
  const child = Bun.spawn(
    [
      Bun.which('bun')!,
      resolve(projectRoot, 'woml-cli/dist/cli.js'),
      'run',
      resolve(temporaryRoot, 'custom-step-workflow.woml'),
      '--state',
      statePath,
    ],
    { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
  );
  const stdoutPromise = new Response(child.stdout).text();
  const stderrPromise = new Response(child.stderr).text();
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const inspection = Bun.spawnSync(
      [
        Bun.which('bun')!,
        resolve(projectRoot, 'woml-cli/dist/cli.js'),
        'list',
        '--state',
        statePath,
        '--json',
      ],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    if (inspection.exitCode === 0) {
      const runs = JSON.parse(inspection.stdout.toString());
      if (runs.items?.some((run: { status: string }) => run.status === 'succeeded')) break;
    }
    await Bun.sleep(50);
  }
  child.kill('SIGINT');
  await child.exited;
  const stdout = await stdoutPromise;
  const stderr = await stderrPromise;
  if (!stdout.includes('{"finalPrice":96}')) {
    throw new Error(`custom-step result was not emitted:\n${stdout}\n${stderr}`);
  }
  const runId = stderr.match(/Run (run_[a-f0-9]+) started/)?.[1];
  if (runId === undefined) throw new Error(`custom-step run ID was not reported:\n${stderr}`);
  const inspected = Bun.spawnSync(
    [
      Bun.which('bun')!,
      resolve(projectRoot, 'woml-cli/dist/cli.js'),
      'get',
      runId,
      '--state',
      statePath,
      '--json',
    ],
    { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
  );
  if (inspected.exitCode !== 0) {
    throw new Error(inspected.stderr.toString());
  }
  const run = JSON.parse(inspected.stdout.toString());
  const reusable = run.reusableDefinitions?.items?.[0];
  if (
    run.profile !== 'woml.run-inspection/v5' ||
    run.status !== 'succeeded' ||
    reusable?.invocationId !== 'discount' ||
    reusable?.status !== 'succeeded' ||
    reusable?.lifecycleStatus !== 'completed'
  ) {
    throw new Error(`custom-step inspection is invalid: ${JSON.stringify(run)}`);
  }
  console.log('Custom-step execution, result threading, lifecycle, and inspection verified.');
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
