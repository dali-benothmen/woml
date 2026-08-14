import { cp, mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const projectRoot = resolve(import.meta.dir, '../..');
const executable = resolve(projectRoot, 'woml-cli/dist/cli.js');
const fixtureRoot = resolve(
  projectRoot,
  'woml/tests/fixtures/reusable-production'
);
const temporaryRoot = await mkdtemp(
  resolve(tmpdir(), 'woml-reusable-production-')
);
const copiedFixtures = resolve(temporaryRoot, 'project');
const environment = {
  ...process.env,
  WOML_SECRETS_PROVIDER: 'env',
  WOML_SECRET_REUSABLE_TEST_TOKEN: 'acceptance-secret-never-log',
};

interface LoggedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stdoutPath: string;
  readonly stderrPath: string;
}

let processIndex = 0;

function start(args: readonly string[]): LoggedProcess {
  processIndex += 1;
  const stdoutPath = resolve(temporaryRoot, `process-${processIndex}.stdout`);
  const stderrPath = resolve(temporaryRoot, `process-${processIndex}.stderr`);
  const child = Bun.spawn([process.execPath, executable, ...args], {
    cwd: copiedFixtures,
    env: environment,
    stdout: Bun.file(stdoutPath),
    stderr: Bun.file(stderrPath),
  });
  return { child, stdoutPath, stderrPath };
}

async function output(process: LoggedProcess): Promise<string> {
  return `${await readFile(process.stdoutPath, 'utf8').catch(() => '')}\n${await readFile(process.stderrPath, 'utf8').catch(() => '')}`;
}

async function waitFor(
  process: LoggedProcess,
  expected: string,
  timeoutMs = 20_000
): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const captured = await output(process);
    if (captured.includes(expected)) return captured;
    if (process.child.exitCode !== null) {
      throw new Error(
        `WOML exited before ${JSON.stringify(expected)}:\n${captured}`
      );
    }
    await Bun.sleep(20);
  }
  throw new Error(`Timed out waiting for ${JSON.stringify(expected)}:\n${await output(process)}`);
}

async function stop(process: LoggedProcess): Promise<void> {
  if (process.child.exitCode === null) process.child.kill('SIGINT');
  const exited = await Promise.race([
    process.child.exited.then(code => ({ code })),
    Bun.sleep(3_000).then(() => undefined),
  ]);
  if (exited === undefined) {
    process.child.kill(9);
    await process.child.exited;
  } else if (exited.code !== 0) {
    throw new Error(`WOML stopped with ${exited.code}:\n${await output(process)}`);
  }
}

function invoke(args: readonly string[]) {
  return Bun.spawnSync([process.execPath, executable, ...args], {
    cwd: copiedFixtures,
    env: environment,
    stdout: 'pipe',
    stderr: 'pipe',
  });
}

function inspection(runId: string, statePath: string) {
  const inspected = invoke(['get', runId, '--state', statePath, '--json']);
  if (inspected.exitCode !== 0) throw new Error(inspected.stderr.toString());
  return JSON.parse(inspected.stdout.toString()) as {
    profile: string;
    status: string;
    reusableDefinitions?: {
      counts: Record<string, number>;
      items: Array<{
        invocationId: string;
        status: string;
        lifecycleStatus: string;
      }>;
    };
  };
}

async function availablePort(): Promise<number> {
  const probe = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = probe.port;
  probe.stop(true);
  if (port === undefined) throw new Error('Bun did not assign an approval port.');
  return port;
}

try {
  await cp(fixtureRoot, copiedFixtures, { recursive: true });

  const compositionState = resolve(temporaryRoot, 'composition.sqlite');
  const composition = start([
    'run',
    resolve(copiedFixtures, 'composition.woml'),
    '--state',
    compositionState,
  ]);
  const compositionOutput = await waitFor(
    composition,
    '{"switchValue":6,"chooseValue":12,"parallelValue":60,"forkValue":120,"secretConfigured":true}'
  );
  if (compositionOutput.includes(environment.WOML_SECRET_REUSABLE_TEST_TOKEN)) {
    throw new Error('A reusable secret leaked into CLI output.');
  }
  const compositionRunId = compositionOutput.match(
    /Run (run_[a-f0-9]+) started/
  )?.[1];
  if (compositionRunId === undefined) {
    throw new Error(`The composition run ID was not reported:\n${compositionOutput}`);
  }
  await stop(composition);
  const composed = inspection(compositionRunId, compositionState);
  if (
    composed.profile !== 'woml.run-inspection/v5' ||
    composed.status !== 'succeeded' ||
    composed.reusableDefinitions?.counts.succeeded !== 7 ||
    composed.reusableDefinitions.items.some(item => item.status !== 'succeeded')
  ) {
    throw new Error(`Reusable composition inspection is invalid: ${JSON.stringify(composed)}`);
  }

  const callFolder = resolve(temporaryRoot, 'call-folder');
  await mkdir(callFolder);
  for (const name of [
    'call-workflow.woml',
    'start-workflow.woml',
    'workflow-caller.woml',
    'calculator.woml',
  ]) {
    await cp(resolve(copiedFixtures, name), resolve(callFolder, name));
  }
  const callRuntime = start([
    'run',
    callFolder,
    '--state',
    resolve(temporaryRoot, 'calls.sqlite'),
  ]);
  const callOutput = await waitFor(callRuntime, 'result: {"called":42,"backgroundRunId":"run_call_');
  if (!callOutput.includes('ready with 2 workflows')) {
    throw new Error(`Reusable definitions were activated as workflows:\n${callOutput}`);
  }
  await stop(callRuntime);

  const approvalState = resolve(temporaryRoot, 'approval.sqlite');
  const approval = start([
    'run',
    resolve(copiedFixtures, 'approval-composition.woml'),
    '--state',
    approvalState,
    '--approval-port',
    String(await availablePort()),
  ]);
  const approvalWaiting = await waitFor(approval, 'Approval URL: http://127.0.0.1:');
  const approvalUrl = approvalWaiting.match(/Approval URL: (http:\/\/127\.0\.0\.1:\d+\/approvals\/\S+)/)?.[1];
  const approvalRunId = approvalWaiting.match(/Run ID: (run_[a-f0-9]+)/)?.[1];
  if (approvalUrl === undefined || approvalRunId === undefined) {
    throw new Error(`Approval capability was not reported:\n${approvalWaiting}`);
  }
  const approvalPage = new URL(approvalUrl);
  const approvalDecision = await fetch(
    `${approvalPage.origin}/api/v1/approvals/${approvalPage.pathname.slice('/approvals/'.length)}/decision`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ decision: 'approved' }),
    }
  );
  if (!approvalDecision.ok) {
    throw new Error(`Approval decision returned ${approvalDecision.status}.`);
  }
  await waitFor(approval, '{"decision":"approved"}');
  await stop(approval);
  const approved = inspection(approvalRunId, approvalState);
  const approvedItems = approved.reusableDefinitions?.items ?? [];
  if (
    approved.status !== 'succeeded' ||
    approvedItems.find(item => item.invocationId === 'approvedScale')?.status !== 'succeeded' ||
    approvedItems.find(item => item.invocationId === 'rejectedScale')?.status !== 'pending'
  ) {
    throw new Error(`Reusable approval-arm inspection is invalid: ${JSON.stringify(approved)}`);
  }

  const cancellationState = resolve(temporaryRoot, 'cancellation.sqlite');
  const cancellation = start([
    'run',
    resolve(copiedFixtures, 'cancellable.woml'),
    '--state',
    cancellationState,
  ]);
  const cancellationStarted = await waitFor(cancellation, 'started under runtime policy');
  const cancelledRunId = cancellationStarted.match(/Run (run_[a-f0-9]+) started/)?.[1];
  if (cancelledRunId === undefined) {
    throw new Error(`The cancellable run ID was not reported:\n${cancellationStarted}`);
  }
  const cancelled = invoke([
    'cancel',
    cancelledRunId,
    '--state',
    cancellationState,
    '--json',
  ]);
  if (cancelled.exitCode !== 0) throw new Error(cancelled.stderr.toString());
  await waitFor(cancellation, `Run ${cancelledRunId} cancelled.`);
  await stop(cancellation);
  const cancelledInspection = inspection(cancelledRunId, cancellationState);
  if (
    cancelledInspection.status !== 'cancelled' ||
    cancelledInspection.reusableDefinitions?.items[0]?.status !== 'cancelled' ||
    cancelledInspection.reusableDefinitions.items[0]?.lifecycleStatus !== 'completed'
  ) {
    throw new Error(`Reusable cancellation inspection is invalid: ${JSON.stringify(cancelledInspection)}`);
  }

  const failureState = resolve(temporaryRoot, 'failure.sqlite');
  const failed = invoke([
    'run',
    resolve(copiedFixtures, 'terminal-failure.woml'),
    '--state',
    failureState,
  ]);
  const failureOutput = `${failed.stdout.toString()}\n${failed.stderr.toString()}`;
  const failedRunId = failureOutput.match(/Run (run_[a-f0-9]+) started/)?.[1];
  if (
    failed.exitCode !== 1 ||
    failedRunId === undefined ||
    !failureOutput.includes('temporary scaling failure') ||
    !failureOutput.includes('Scaling failed: WOML_SCRIPT_THROWN') ||
    !failureOutput.includes('WOML runtime error [WOML_SCRIPT_FAILED]')
  ) {
    throw new Error(`Reusable terminal failure lost its original result or lifecycle:\n${failureOutput}`);
  }
  const failedInspection = inspection(failedRunId, failureState);
  if (
    failedInspection.status !== 'failed' ||
    failedInspection.reusableDefinitions?.items[0]?.status !== 'failed' ||
    failedInspection.reusableDefinitions.items[0]?.lifecycleStatus !== 'completed'
  ) {
    throw new Error(`Reusable terminal failure inspection is invalid: ${JSON.stringify(failedInspection)}`);
  }

  const timedOut = invoke([
    'run',
    resolve(copiedFixtures, 'workflow-timeout.woml'),
    '--state',
    resolve(temporaryRoot, 'timeout.sqlite'),
  ]);
  if (
    timedOut.exitCode !== 1 ||
    !timedOut.stderr.toString().includes('WOML_WORKFLOW_TIMED_OUT')
  ) {
    throw new Error(
      `Reusable workflow timeout did not fail safely:\n${timedOut.stdout.toString()}\n${timedOut.stderr.toString()}`
    );
  }

  const backupPath = resolve(temporaryRoot, 'backup');
  const backup = invoke([
    'backup',
    backupPath,
    '--state',
    compositionState,
    '--json',
  ]);
  if (backup.exitCode !== 0 || JSON.parse(backup.stdout.toString()).verified !== true) {
    throw new Error(`Reusable backup failed:\n${backup.stderr.toString()}`);
  }
  const restoredState = resolve(temporaryRoot, 'restored.sqlite');
  const restored = invoke([
    'restore',
    backupPath,
    '--state',
    restoredState,
    '--json',
  ]);
  if (restored.exitCode !== 0) throw new Error(restored.stderr.toString());
  const restoredInspection = inspection(compositionRunId, restoredState);
  if (
    restoredInspection.status !== 'succeeded' ||
    restoredInspection.reusableDefinitions?.counts.succeeded !== 7
  ) {
    throw new Error(`Reusable artifacts were not restored: ${JSON.stringify(restoredInspection)}`);
  }

  console.log(
    'Reusable composition, workflow calls, approval arms, terminal failure, cancellation, timeout, inspection, and backup/restore verified.'
  );
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
