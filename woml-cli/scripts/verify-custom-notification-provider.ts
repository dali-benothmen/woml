import { copyFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

const relayPort = 7360;
const approvalPort = 7361;
const directory = await mkdtemp(
  resolve(tmpdir(), 'woml-custom-provider-acceptance-')
);
let decisionError: unknown;

async function approveWithRetry(url: string): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    await Bun.sleep(25);
    const response = await fetch(url, { method: 'POST' });
    if (response.ok) return;
    if (response.status !== 500) {
      throw new Error(`Approval capability returned ${response.status}.`);
    }
  }
  throw new Error('Approval capability did not become ready.');
}

const relay = Bun.serve({
  hostname: '127.0.0.1',
  port: relayPort,
  async fetch(request) {
    if (request.method !== 'POST' || new URL(request.url).pathname !== '/notify') {
      return new Response('Not found', { status: 404 });
    }
    const body = (await request.json()) as { approve?: unknown };
    if (typeof body.approve !== 'string') {
      return new Response('Invalid capability', { status: 400 });
    }
    void approveWithRetry(body.approve).catch(error => {
      decisionError = error;
    });
    return Response.json({ accepted: true }, { status: 202 });
  },
});

try {
  const fixtureRoot = resolve(
    import.meta.dir,
    '../../woml/tests/fixtures/reusable-definitions'
  );
  const workflow = resolve(directory, 'local-provider-approval-workflow.woml');
  await Promise.all([
    copyFile(
      resolve(fixtureRoot, 'local-provider-approval-workflow.woml'),
      workflow
    ),
    copyFile(
      resolve(fixtureRoot, 'local-approval-provider.woml'),
      resolve(directory, 'local-approval-provider.woml')
    ),
  ]);
  const stdoutPath = resolve(directory, 'stdout.log');
  const stderrPath = resolve(directory, 'stderr.log');
  const child = Bun.spawn(
    [
      process.execPath,
      resolve(import.meta.dir, '../dist/cli.js'),
      'run',
      workflow,
      '--state',
      resolve(directory, 'state.sqlite'),
      '--approval-port',
      String(approvalPort),
    ],
    { stdout: Bun.file(stdoutPath), stderr: Bun.file(stderrPath) }
  );
  let capturedStdout = '';
  for (let attempt = 0; attempt < 400; attempt += 1) {
    capturedStdout = await Bun.file(stdoutPath).text();
    if (capturedStdout.trim().length > 0) break;
    if (child.exitCode !== null) break;
    await Bun.sleep(25);
  }
  const timedOut = capturedStdout.trim().length === 0 && child.exitCode === null;
  if (child.exitCode === null) {
    child.kill('SIGINT');
    const stopped = await Promise.race([
      child.exited.then(() => true),
      Bun.sleep(2_000).then(() => false),
    ]);
    if (!stopped) child.kill(9);
  }
  const [stdout, stderr] = await Promise.all([
    capturedStdout.length > 0
      ? Promise.resolve(capturedStdout)
      : Bun.file(stdoutPath).text(),
    Bun.file(stderrPath).text(),
  ]);
  if (timedOut || stdout.trim().length === 0) {
    throw new Error(
      `WOML did not produce a result within 20 seconds.\nstdout:\n${stdout}\nstderr:\n${stderr}`
    );
  }
  if (decisionError !== undefined) throw decisionError;
  const result = JSON.parse(stdout.trim()) as unknown;
  if (
    JSON.stringify(result) !==
    JSON.stringify({ orderId: 'order-42', approved: true })
  ) {
    throw new Error(`Unexpected workflow result: ${stdout.trim()}`);
  }
  if (!stderr.includes('WOML workflow is waiting for approval.')) {
    throw new Error('The approval journey was not observable in CLI output.');
  }
  console.log('Custom notification provider acceptance passed.');
} finally {
  relay.stop(true);
  await rm(directory, { recursive: true, force: true });
}
