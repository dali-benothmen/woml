import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const approvalFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'approval.woml'
);
let temporaryDirectory: string;

async function availablePort(): Promise<number> {
  const probe = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = probe.port;
  probe.stop(true);
  if (port === undefined) throw new Error('Bun did not assign a test port.');
  return port;
}

function spawnApproval(args: readonly string[], cwd = projectRoot) {
  const child = Bun.spawn([cliPath, ...args], {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const stdout = new Response(child.stdout).text();
  let resolveUrl!: (url: string) => void;
  let rejectUrl!: (error: Error) => void;
  let announced = false;
  const url = new Promise<string>((resolve, reject) => {
    resolveUrl = resolve;
    rejectUrl = reject;
  });
  const stderr = (async () => {
    const reader = child.stderr.getReader();
    const decoder = new TextDecoder();
    let text = '';
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      text += decoder.decode(value, { stream: true });
      const match = text.match(
        /Approval URL: (http:\/\/127\.0\.0\.1:\d+\/approvals\/\S+)/
      );
      if (!announced && match !== null) {
        announced = true;
        resolveUrl(match[1]);
      }
    }
    text += decoder.decode();
    if (!announced) {
      rejectUrl(new Error(`CLI ended before announcing approval:\n${text}`));
    }
    return text;
  })();
  return { child, stdout, stderr, url };
}

function decisionEndpoint(url: string): string {
  const page = new URL(url);
  return `${page.origin}/api/v1/approvals/${page.pathname.slice(
    '/approvals/'.length
  )}/decision`;
}

async function postDecision(url: string, decision: 'approved' | 'rejected') {
  return fetch(decisionEndpoint(url), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ decision }),
  });
}

beforeAll(async () => {
  const build = Bun.spawnSync([Bun.which('bun')!, 'run', 'build'], {
    cwd: packageRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (build.exitCode !== 0) {
    throw new Error(
      `Could not build the CLI:\n${build.stdout.toString()}${build.stderr.toString()}`
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-cli-approval-a6-'));
});

afterAll(async () => {
  await rm(temporaryDirectory, { recursive: true, force: true });
});

describe('woml run Human Approval', () => {
  for (const decision of ['approved', 'rejected'] as const) {
    test(`runs the real ${decision} browser/API journey`, async () => {
      const defaultDirectory = join(temporaryDirectory, 'default-product-path');
      const explicitPort = await availablePort();
      if (decision === 'approved') await mkdir(defaultDirectory);
      const running =
        decision === 'approved'
          ? spawnApproval(['run', approvalFixturePath], defaultDirectory)
          : spawnApproval([
              'run',
              approvalFixturePath,
              '--state',
              join(temporaryDirectory, `${decision}.sqlite`),
              '--approval-port',
              String(explicitPort),
            ]);
      const url = await running.url;
      if (decision === 'approved') {
        expect(new URL(url).port).toBe('7331');
      }
      const page = await fetch(url);
      expect(page.status).toBe(200);
      expect(await page.text()).toContain('Editorial approval');

      const response = await postDecision(url, decision);
      expect(response.status).toBe(200);
      expect((await response.json()).decision).toBe(decision);

      const [stdout, stderr, exitCode] = await Promise.all([
        running.stdout,
        running.stderr,
        running.child.exited,
      ]);
      expect(exitCode).toBe(0);
      expect(JSON.parse(stdout)).toEqual({
        decision,
        source: 'human',
        published: decision === 'approved',
      });
      expect(stderr).toContain('waiting for human approval');
      expect(stderr).toContain(`Run ID: run_`);
      expect(stderr).toContain(`Approval URL: ${url}`);
      expect(stdout).not.toContain('/approvals/');
      if (decision === 'approved') {
        expect(
          (await stat(join(defaultDirectory, '.woml', 'state.sqlite'))).isFile()
        ).toBe(true);
      }
    });
  }

  test('recovers a stopped waiting run with a newly issued URL', async () => {
    const statePath = join(temporaryDirectory, 'resume.sqlite');
    const firstPort = await availablePort();
    const first = spawnApproval([
      'run',
      approvalFixturePath,
      '--state',
      statePath,
      '--approval-port',
      String(firstPort),
    ]);
    const firstUrl = await first.url;
    first.child.kill();
    const firstStderr = await first.stderr;
    await first.child.exited;
    const durableRunId = firstStderr.match(/Run ID: (run_[A-Za-z0-9_-]+)/)?.[1];
    expect(durableRunId).toBeDefined();

    const changedWorkflowPath = join(
      temporaryDirectory,
      'changed-approval.woml'
    );
    await writeFile(
      changedWorkflowPath,
      (await Bun.file(approvalFixturePath).text()).replace(
        'WOML reaches human review',
        'A different durable definition'
      )
    );
    const mismatched = Bun.spawn(
      [
        cliPath,
        'run',
        changedWorkflowPath,
        '--state',
        statePath,
        '--resume',
        durableRunId!,
        '--approval-port',
        String(await availablePort()),
      ],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    const [mismatchOutput, mismatchError, mismatchExit] = await Promise.all([
      new Response(mismatched.stdout).text(),
      new Response(mismatched.stderr).text(),
      mismatched.exited,
    ]);
    expect(mismatchOutput).toBe('');
    expect(mismatchError).toContain(
      'does not match the durable run definition'
    );
    expect(mismatchExit).toBe(1);

    const secondPort = await availablePort();
    const resumed = spawnApproval([
      'run',
      approvalFixturePath,
      '--state',
      statePath,
      '--resume',
      durableRunId!,
      '--approval-port',
      String(secondPort),
    ]);
    const secondUrl = await resumed.url;
    expect(secondUrl).not.toBe(firstUrl);
    expect((await postDecision(secondUrl, 'approved')).status).toBe(200);
    expect(await resumed.stdout).toBe(
      '{"decision":"approved","source":"human","published":true}\n'
    );
    expect(await resumed.child.exited).toBe(0);
    await resumed.stderr;
  });

  test('settles a deadline and continues the rejected route automatically', async () => {
    const workflowPath = join(temporaryDirectory, 'timeout-reject.woml');
    await writeFile(
      workflowPath,
      (await Bun.file(approvalFixturePath).text()).replace(
        'timeout="24h"',
        'timeout="100ms"'
      )
    );
    const port = await availablePort();
    const child = Bun.spawn(
      [
        cliPath,
        'run',
        workflowPath,
        '--state',
        join(temporaryDirectory, 'timeout-reject.sqlite'),
        '--approval-port',
        String(port),
      ],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]);
    expect(exitCode).toBe(0);
    expect(JSON.parse(stdout)).toEqual({
      decision: 'rejected',
      source: 'timeout',
      published: false,
    });
    expect(stderr).toContain('(reject on timeout)');
  });

  test('reports a deterministic error when the configured port is busy', async () => {
    const port = await availablePort();
    const occupied = Bun.serve({
      hostname: '127.0.0.1',
      port,
      fetch: () => new Response(),
    });
    try {
      const child = Bun.spawn(
        [
          cliPath,
          'run',
          approvalFixturePath,
          '--state',
          join(temporaryDirectory, 'port-conflict.sqlite'),
          '--approval-port',
          String(port),
        ],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      const [stdout, stderr, exitCode] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ]);
      expect(stdout).toBe('');
      expect(stderr).toContain('WOML_APPROVAL_SERVER_BIND_FAILED');
      expect(stderr).toContain(`127.0.0.1:${port}`);
      expect(exitCode).toBe(1);
    } finally {
      occupied.stop(true);
    }
  });
});
