import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { Database } from 'bun:sqlite';
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
  const commandArgs = args[0] === 'run' ? ['test', ...args.slice(1)] : args;
  const child = Bun.spawn([cliPath, ...commandArgs], {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const stdout = new Response(child.stdout).text();
  const queuedUrls: string[] = [];
  const seenUrls = new Set<string>();
  const waiters: Array<{
    resolve: (url: string) => void;
    reject: (error: Error) => void;
  }> = [];
  let ended = false;
  let finalStderr = '';
  const nextUrl = (): Promise<string> => {
    const queued = queuedUrls.shift();
    if (queued !== undefined) return Promise.resolve(queued);
    if (ended) {
      return Promise.reject(
        new Error(
          `CLI ended before announcing another approval:\n${finalStderr}`
        )
      );
    }
    return new Promise<string>((resolve, reject) => {
      waiters.push({ resolve, reject });
    });
  };
  const announce = (url: string) => {
    if (seenUrls.has(url)) return;
    seenUrls.add(url);
    const waiter = waiters.shift();
    if (waiter === undefined) queuedUrls.push(url);
    else waiter.resolve(url);
  };
  const stderr = (async () => {
    const reader = child.stderr.getReader();
    const decoder = new TextDecoder();
    let text = '';
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      text += decoder.decode(value, { stream: true });
      for (const match of text.matchAll(
        /Approval URL: (http:\/\/127\.0\.0\.1:\d+\/approvals\/\S+)/g
      )) {
        announce(match[1]);
      }
    }
    text += decoder.decode();
    ended = true;
    finalStderr = text;
    for (const waiter of waiters.splice(0)) {
      waiter.reject(
        new Error(`CLI ended before announcing another approval:\n${text}`)
      );
    }
    return text;
  })();
  return { child, stdout, stderr, url: nextUrl(), nextUrl };
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
}, 120_000);

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('woml test Human Approval', () => {
  for (const decision of ['approved', 'rejected'] as const) {
    test(`runs the real ${decision} browser/API journey`, async () => {
      const defaultDirectory = join(temporaryDirectory, 'default-product-path');
      const explicitPort = await availablePort();
      if (decision === 'approved') await mkdir(defaultDirectory);
      const statePath =
        decision === 'approved'
          ? join(defaultDirectory, '.woml', 'state.sqlite')
          : join(temporaryDirectory, `${decision}.sqlite`);
      const running =
        decision === 'approved'
          ? spawnApproval(['run', approvalFixturePath], defaultDirectory)
          : spawnApproval([
              'run',
              approvalFixturePath,
              '--state',
              statePath,
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

      const token = new URL(url).pathname.slice('/approvals/'.length);
      const secret = token.slice(token.indexOf('.') + 1);
      for (const databaseFile of [statePath, `${statePath}-wal`]) {
        if (!(await Bun.file(databaseFile).exists())) continue;
        const bytes = Buffer.from(await Bun.file(databaseFile).arrayBuffer());
        expect(bytes.includes(Buffer.from(token))).toBe(false);
        expect(bytes.includes(Buffer.from(secret))).toBe(false);
      }

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

  test('routes a Model v6 approval workflow back through its selected retrying arm', async () => {
    const workflowPath = join(
      temporaryDirectory,
      'approval-retry-model-v6.woml'
    );
    const statePath = join(temporaryDirectory, 'approval-retry-model-v6.sqlite');
    await writeFile(
      workflowPath,
      `<woml>
<workflow version="0.1" id="approval-retry-model-v6">
  <triggers><manual id="start" /></triggers>
  <steps>
    <approval id="review">
      <when-approved>
        <step id="publish" retry="2" retry-backoff="fixed" retry-delay="1ms">
          <script>
            if (attempt.number === 1) throw new Error("temporary publish failure");
            return { published: true };
          </script>
        </step>
      </when-approved>
      <when-rejected />
    </approval>
    <step id="finish">
      <script>
        return {
          decision: context.steps.review.decision,
          published: context.steps.publish.published
        };
      </script>
    </step>
  </steps>
</workflow>
</woml>`
    );

    const running = spawnApproval([
      'run',
      workflowPath,
      '--state',
      statePath,
      '--approval-port',
      String(await availablePort()),
    ]);
    const url = await running.url;
    expect((await postDecision(url, 'approved')).status).toBe(200);

    const [stdout, stderr, exitCode] = await Promise.all([
      running.stdout,
      running.stderr,
      running.child.exited,
    ]);
    expect(exitCode).toBe(0);
    expect(JSON.parse(stdout)).toEqual({
      decision: 'approved',
      published: true,
    });
    expect(stderr.match(/waiting for human approval/g)).toHaveLength(1);
  });

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
        'test',
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

  test('supports approval at the beginning and as the terminal workflow action', async () => {
    const cases = [
      {
        name: 'beginning',
        source: `<woml>
<workflow version="0.1" id="approval-beginning">
  <triggers><manual id="start" /></triggers>
  <steps>
    <approval id="review"><when-approved /><when-rejected /></approval>
    <step id="finish"><script>return { decision: context.steps.review.decision, position: "beginning" };</script></step>
  </steps>
</workflow>
</woml>`,
        expected: { decision: 'approved', position: 'beginning' },
      },
      {
        name: 'terminal',
        source: `<woml>
<workflow version="0.1" id="approval-terminal">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="prepare"><script>return { ready: true };</script></step>
    <approval id="review"><when-approved /><when-rejected /></approval>
  </steps>
</workflow>
</woml>`,
        expected: { decision: 'approved', source: 'human' },
      },
    ] as const;

    for (const item of cases) {
      const workflowPath = join(temporaryDirectory, `${item.name}.woml`);
      await writeFile(workflowPath, item.source);
      const running = spawnApproval([
        'test',
        workflowPath,
        '--state',
        join(temporaryDirectory, `${item.name}.sqlite`),
        '--approval-port',
        String(await availablePort()),
      ]);
      expect((await postDecision(await running.url, 'approved')).status).toBe(
        200
      );
      const [stdout, stderr, exitCode] = await Promise.all([
        running.stdout,
        running.stderr,
        running.child.exited,
      ]);
      if (exitCode !== 0) {
        throw new Error(`${item.name} approval failed:\n${stderr}`);
      }
      expect(JSON.parse(stdout)).toMatchObject(item.expected);
    }
  });

  test('composes a selected approval with branch and parallel while skipping the unselected approval', async () => {
    const workflowPath = join(temporaryDirectory, 'approval-composition.woml');
    await writeFile(
      workflowPath,
      `<woml>
<workflow version="0.1" id="approval-composition">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="prepare" concurrency="2" on-error="wait-all">
      <step id="routeFlag"><script>return { review: true };</script></step>
      <step id="preparedValue"><script>return { value: 40 };</script></step>
    </parallel>
    <branch id="route">
      <when test="{{context.steps.routeFlag.review}}">
        <approval id="selectedApproval" name="Selected approval">
          <when-approved><step id="selectedResult"><script>return { selected: true };</script></step></when-approved>
          <when-rejected><step id="selectedRejected"><script>return { selected: false };</script></step></when-rejected>
        </approval>
        <result value="{{context.steps.selectedApproval}}" />
      </when>
      <otherwise>
        <approval id="unselectedApproval" name="Must never wait">
          <when-approved /><when-rejected />
        </approval>
        <result value="{{context.steps.unselectedApproval}}" />
      </otherwise>
    </branch>
    <parallel id="afterApproval" concurrency="2" on-error="wait-all">
      <step id="addOne"><script>return { value: context.steps.preparedValue.value + 1 };</script></step>
      <step id="addTwo"><script>return { value: context.steps.preparedValue.value + 2 };</script></step>
    </parallel>
    <step id="finish"><script>return { decision: context.steps.route.decision, total: context.steps.addOne.value + context.steps.addTwo.value };</script></step>
  </steps>
</workflow>
</woml>`
    );
    const running = spawnApproval([
      'run',
      workflowPath,
      '--state',
      join(temporaryDirectory, 'approval-composition.sqlite'),
      '--approval-port',
      String(await availablePort()),
    ]);
    const url = await running.url;
    const page = await (await fetch(url)).text();
    expect(page).toContain('Selected approval');
    expect(page).not.toContain('Must never wait');
    expect((await postDecision(url, 'approved')).status).toBe(200);
    expect(JSON.parse(await running.stdout)).toEqual({
      decision: 'approved',
      total: 83,
    });
    expect(await running.child.exited).toBe(0);
    const stderr = await running.stderr;
    expect(stderr.match(/Approval URL:/g)).toHaveLength(1);
  });

  test('handles nested approvals and two durable waiting cycles in one run', async () => {
    const workflowPath = join(temporaryDirectory, 'nested-approvals.woml');
    await writeFile(
      workflowPath,
      `<woml>
<workflow version="0.1" id="nested-approvals">
  <triggers><manual id="start" /></triggers>
  <steps>
    <approval id="editorial" name="Editorial approval">
      <when-approved>
        <step id="editorialRecorded"><script>return { recorded: true };</script></step>
        <approval id="legal" name="Legal approval">
          <when-approved>
            <step id="legalCheck"><script>return { passed: true };</script></step>
            <step id="legalRecorded"><script>return { count: 2 };</script></step>
          </when-approved>
          <when-rejected />
        </approval>
      </when-approved>
      <when-rejected />
    </approval>
    <step id="finish"><script>return { editorial: context.steps.editorial.decision, legal: context.steps.legal.decision, actions: context.steps.legalRecorded.count };</script></step>
  </steps>
</workflow>
</woml>`
    );
    const running = spawnApproval([
      'run',
      workflowPath,
      '--state',
      join(temporaryDirectory, 'nested-approvals.sqlite'),
      '--approval-port',
      String(await availablePort()),
    ]);
    const editorialUrl = await running.url;
    expect(await (await fetch(editorialUrl)).text()).toContain(
      'Editorial approval'
    );
    expect((await postDecision(editorialUrl, 'approved')).status).toBe(200);
    const legalUrl = await running.nextUrl();
    expect(legalUrl).not.toBe(editorialUrl);
    expect(await (await fetch(legalUrl)).text()).toContain('Legal approval');
    expect((await postDecision(legalUrl, 'approved')).status).toBe(200);

    expect(JSON.parse(await running.stdout)).toEqual({
      editorial: 'approved',
      legal: 'approved',
      actions: 2,
    });
    expect(await running.child.exited).toBe(0);
    expect((await running.stderr).match(/Approval URL:/g)).toHaveLength(2);
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
        'test',
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
    const statePath = join(temporaryDirectory, 'port-conflict.sqlite');
    const occupied = Bun.serve({
      hostname: '127.0.0.1',
      port,
      fetch: () => new Response(),
    });
    try {
      const child = Bun.spawn(
        [
          cliPath,
          'test',
          approvalFixturePath,
          '--state',
          statePath,
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

    const database = new Database(statePath, { readonly: true });
    const row = database
      .query<
        { run_id: string },
        []
      >('SELECT run_id FROM woml_runs ORDER BY created_at DESC LIMIT 1')
      .get();
    database.close();
    expect(row?.run_id).toStartWith('run_');

    const resumed = spawnApproval([
      'run',
      approvalFixturePath,
      '--state',
      statePath,
      '--resume',
      row!.run_id,
      '--approval-port',
      String(await availablePort()),
    ]);
    const recoveredUrl = await resumed.url;
    expect((await postDecision(recoveredUrl, 'approved')).status).toBe(200);
    expect(JSON.parse(await resumed.stdout)).toMatchObject({
      decision: 'approved',
      published: true,
    });
    expect(await resumed.child.exited).toBe(0);
    await resumed.stderr;
  });
});
