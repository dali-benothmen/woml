import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { Database } from 'bun:sqlite';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const helloFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'hello.woml'
);
const branchFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'branch.woml'
);
const parallelFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'parallel.woml'
);
const approvalFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'approval.woml'
);
const retryFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'retry.woml'
);
const retryCompositionFixturePath = join(
  import.meta.dir,
  '..',
  '..',
  'woml',
  'tests',
  'fixtures',
  'retry-composition.woml'
);
let temporaryDirectory: string;

interface CommandResult {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number;
}

async function runCli(...args: string[]): Promise<CommandResult> {
  const process = Bun.spawn([cliPath, ...args], {
    cwd: projectRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });

  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(process.stdout).text(),
    new Response(process.stderr).text(),
    process.exited,
  ]);

  return { stdout, stderr, exitCode };
}

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

function spawnPackagedApproval(
  executable: string,
  args: readonly string[],
  cwd: string
) {
  const child = Bun.spawn([executable, ...args], {
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
      rejectUrl(new Error(`Packaged CLI did not announce approval:\n${text}`));
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
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-cli-phase4-'));
});

afterAll(async () => {
  await rm(temporaryDirectory, { recursive: true, force: true });
});

describe('woml run', () => {
  test('has no production dependency on the TypeScript workflow executor', async () => {
    const source = await Bun.file(join(packageRoot, 'src', 'cli.ts')).text();
    expect(source).toContain('executeWorkflowWithRust');
    expect(source).not.toMatch(/\bexecuteWorkflow\s*\(/);
  });

  test('runs hello.woml through the public executable', async () => {
    const expected = JSON.parse(
      await Bun.file(
        join(packageRoot, 'tests', 'fixtures', 'hello.cli.v0.1.json')
      ).text()
    );

    const result = await runCli('run', helloFixturePath);

    expect(result).toEqual({
      stdout: expected.stdout,
      stderr: expected.stderr,
      exitCode: expected.exitCode,
    });
  });

  test(
    'runs retry.woml durably through attempts 1, 2, and 3',
    async () => {
      const statePath = join(temporaryDirectory, 'retry-state.sqlite');
      const result = await runCli(
        'run',
        retryFixturePath,
        '--state',
        statePath
      );

      expect(result.stdout).toBe('{"message":"Hello World"}\n');
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toContain(
        'Step greet failed (attempt 1/3): WOML_SCRIPT_THROWN\n'
      );
      expect(result.stderr).toContain('Retry 2/3 scheduled in 1s.\n');
      expect(result.stderr).toContain(
        `Recovery: woml run ${JSON.stringify(retryFixturePath)} --state ${JSON.stringify(
          statePath
        )} --resume "run_`
      );
      expect(result.stderr).toContain(
        'Step greet failed (attempt 2/3): WOML_SCRIPT_THROWN\n'
      );
      expect(result.stderr).toContain('Retry 3/3 scheduled in 2s.\n');
      expect(result.stderr).toContain(
        'Step greet succeeded on attempt 3/3.\n'
      );
      expect((await stat(statePath)).isFile()).toBe(true);
    },
    10_000
  );

  test(
    'resumes a durably scheduled retry without replaying attempt 1',
    async () => {
      const statePath = join(temporaryDirectory, 'retry-resume-state.sqlite');
      const child = Bun.spawn(
        [cliPath, 'run', retryFixturePath, '--state', statePath],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      const reader = child.stderr.getReader();
      const decoder = new TextDecoder();
      let firstStderr = '';
      while (!firstStderr.includes('Retry 2/3 scheduled')) {
        const chunk = await reader.read();
        if (chunk.done) break;
        firstStderr += decoder.decode(chunk.value, { stream: true });
      }
      child.kill();
      await child.exited;
      reader.releaseLock();
      expect(firstStderr).toContain(
        'Step greet failed (attempt 1/3): WOML_SCRIPT_THROWN'
      );

      const database = new Database(statePath, { readonly: true });
      const row = database
        .query('SELECT run_id AS runId FROM woml_runs LIMIT 1')
        .get() as { runId: string };
      const eventRows = database
        .query(
          'SELECT event_json AS eventJson FROM woml_run_events WHERE run_id = ? ORDER BY sequence'
        )
        .all(row.runId) as { eventJson: string }[];
      database.close();
      const attemptOneStarts = eventRows
        .map(event => JSON.parse(event.eventJson) as Record<string, unknown>)
        .filter(event => {
          const data = event.data as Record<string, unknown> | undefined;
          return (
            event.type === 'step_attempt_started' &&
            data?.nodeId === 'greet' &&
            data.attempt === 1
          );
        });
      expect(attemptOneStarts).toHaveLength(1);

      const resumed = await runCli(
        'run',
        retryFixturePath,
        '--state',
        statePath,
        '--resume',
        row.runId
      );
      expect(resumed.exitCode).toBe(0);
      expect(resumed.stdout).toBe('{"message":"Hello World"}\n');
      expect(resumed.stderr).not.toContain('attempt 1/3');
      expect(resumed.stderr).toContain(
        'Step greet succeeded on attempt 3/3.\n'
      );
    },
    10_000
  );

  test(
    'reports source-aware retry exhaustion after the final attempt',
    async () => {
      const workflowPath = join(temporaryDirectory, 'retry-exhausted.woml');
      const statePath = join(temporaryDirectory, 'retry-exhausted.sqlite');
      const source = (await Bun.file(retryFixturePath).text()).replace(
        'if (attempt.number < 3)',
        'if (true)'
      );
      await writeFile(workflowPath, source);

      const result = await runCli(
        'run',
        workflowPath,
        '--state',
        statePath
      );
      expect(result.exitCode).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain(
        'Step greet failed (attempt 3/3): WOML_SCRIPT_THROWN\n'
      );
      expect(result.stderr).toMatch(
        new RegExp(
          `WOML runtime error \\[WOML_STEP_RETRIES_EXHAUSTED\\] at ${workflowPath.replaceAll('.', '\\.')}:\\d+:\\d+ \\(step "greet"\\): attempt 3 of 3 failed \\[WOML_SCRIPT_THROWN\\]\\.`
        )
      );
    },
    10_000
  );

  test(
    'runs retry inside parallel inside a selected branch',
    async () => {
      const statePath = join(
        temporaryDirectory,
        'retry-composition-state.sqlite'
      );
      const result = Bun.spawnSync([
        process.execPath,
        cliPath,
        'run',
        retryCompositionFixturePath,
        '--state',
        statePath,
      ]);

      expect(result.exitCode).toBe(0);
      expect(result.stderr.toString()).toContain(
        'Step leftCheck failed (attempt 1/2): WOML_SCRIPT_THROWN\n'
      );
      expect(result.stderr.toString()).toContain(
        'Step rightCheck failed (attempt 1/3): WOML_SCRIPT_THROWN\n'
      );
      expect(result.stderr.toString()).toContain(
        'Step leftCheck succeeded on attempt 2/2.\n'
      );
      expect(result.stderr.toString()).toContain(
        'Step rightCheck succeeded on attempt 3/3.\n'
      );
      expect(result.stdout.toString()).toBe('{"total":42}\n');
      expect((await stat(statePath)).isFile()).toBe(true);
    },
    10_000
  );

  test('runs the selected when route through the public executable', async () => {
    const result = await runCli('run', branchFixturePath);

    expect(result).toEqual({
      stdout: '{"message":"Final status: reviewed"}\n',
      stderr: '',
      exitCode: 0,
    });
  });

  test('runs otherwise when no condition is true', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-otherwise.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      'needsReview: true',
      'needsReview: false'
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result).toEqual({
      stdout: '{"message":"Final status: accepted-automatically"}\n',
      stderr: '',
      exitCode: 0,
    });
  });

  test('runs parallel.woml through the public executable', async () => {
    const expected = JSON.parse(
      await Bun.file(
        join(packageRoot, 'tests', 'fixtures', 'parallel.cli.v0.1.json')
      ).text()
    );

    const result = await runCli('run', parallelFixturePath);

    expect(result).toEqual({
      stdout: expected.stdout,
      stderr: expected.stderr,
      exitCode: expected.exitCode,
    });
  });

  test('runs a one-child parallel group as a valid trivial group', async () => {
    const workflowPath = join(temporaryDirectory, 'parallel-one-child.woml');
    await writeFile(
      workflowPath,
      `<workflow version="0.1" id="one-child-parallel">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="checks" concurrency="1" on-error="wait-all">
      <step id="onlyCheck"><script>return { value: 42 };</script></step>
    </parallel>
    <step id="finish"><script>return { value: context.steps.onlyCheck.value };</script></step>
  </steps>
</workflow>`
    );

    expect(await runCli('run', workflowPath)).toEqual({
      stdout: '{"value":42}\n',
      stderr: '',
      exitCode: 0,
    });
  });

  test('gives every parallel child the same pre-fork context', async () => {
    const workflowPath = join(temporaryDirectory, 'parallel-snapshot.woml');
    await writeFile(
      workflowPath,
      `<workflow version="0.1" id="parallel-snapshot">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="checks" concurrency="2" on-error="wait-all">
      <step id="probe"><script>await new Promise(resolve => setTimeout(resolve, 50)); return { sawSibling: context.steps.sibling !== undefined };</script></step>
      <step id="sibling"><script>return { completed: true };</script></step>
    </parallel>
    <step id="finish"><script>return { sawSibling: context.steps.probe.sawSibling };</script></step>
  </steps>
</workflow>`
    );

    expect(await runCli('run', workflowPath)).toEqual({
      stdout: '{"sawSibling":false}\n',
      stderr: '',
      exitCode: 0,
    });
  });

  for (const policy of ['wait-all', 'fail-fast'] as const) {
    test(`maps a ${policy} child failure to its original script`, async () => {
      const workflowPath = join(
        temporaryDirectory,
        `parallel-${policy}-failure.woml`
      );
      const source = (await Bun.file(parallelFixturePath).text())
        .replace('on-error="wait-all"', `on-error="${policy}"`)
        .replace(
          'await new Promise(resolve => setTimeout(resolve, 80));\n          return {\n            fieldId: context.steps.loadField.fieldId,\n            temperature: 22\n          };',
          'throw new Error("weather unavailable");'
        );
      await writeFile(workflowPath, source);

      const result = await runCli('run', workflowPath);

      expect(result.stdout).toBe('');
      expect(result.stderr).toContain(
        'WOML runtime error [WOML_PARALLEL_CHILD_FAILED]'
      );
      expect(result.stderr).toContain(`${workflowPath}:15:`);
      expect(result.stderr).toContain(
        'step "loadWeather" in parallel "fieldData"'
      );
      expect(result.exitCode).toBe(1);
    });
  }

  test('rejects invalid parallel concurrency through woml run', async () => {
    const workflowPath = join(
      temporaryDirectory,
      'parallel-invalid-concurrency.woml'
    );
    const source = (await Bun.file(parallelFixturePath).text()).replace(
      'concurrency="2"',
      'concurrency="3"'
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML validation error [WOML_PARALLEL_INVALID_CONCURRENCY]'
    );
    expect(result.stderr).toContain(`${workflowPath}:13:`);
    expect(result.exitCode).toBe(1);
  });

  test('rejects a terminal parallel group through woml run', async () => {
    const workflowPath = join(temporaryDirectory, 'parallel-terminal.woml');
    await writeFile(
      workflowPath,
      `<workflow version="0.1" id="terminal-parallel">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="checks">
      <step id="check"><script>return { ok: true };</script></step>
    </parallel>
  </steps>
</workflow>`
    );

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML validation error [WOML_PARALLEL_TERMINAL_UNSUPPORTED]'
    );
    expect(result.stderr).toContain(`${workflowPath}:4:`);
    expect(result.exitCode).toBe(1);
  });

  test('rejects invalid command arguments with usage and exit code 2', async () => {
    const result = await runCli('hello.woml');

    expect(result.stdout).toBe('');
    expect(result.stderr).toBe(
      'Usage: woml run <workflow.woml> [--state <path>] [--resume <runId>] [--approval-port <port>]\n'
    );
    expect(result.exitCode).toBe(2);
  });

  test('reports a missing workflow file without printing success output', async () => {
    const result = await runCli('run', 'missing.woml');

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML input error [WOML_FILE_NOT_FOUND]');
    expect(result.stderr).toContain('missing.woml');
    expect(result.exitCode).toBe(1);
  });

  test('reports source diagnostics with phase, file, line, and column', async () => {
    const workflowPath = join(temporaryDirectory, 'invalid.woml');
    await writeFile(workflowPath, '<workflow>');

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML parse error');
    expect(result.stderr).toContain(`${workflowPath}:1:`);
    expect(result.exitCode).toBe(1);
  });

  test('reports script failures with the runtime phase and step ID', async () => {
    const workflowPath = join(temporaryDirectory, 'failure.woml');
    await writeFile(
      workflowPath,
      `<workflow version="1.0.0" id="failure">
  <triggers><manual id="start" /></triggers>
  <steps><step id="broken"><script>throw new Error("boom");</script></step></steps>
</workflow>`
    );

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML runtime error [WOML_SCRIPT_FAILED]');
    expect(result.stderr).toContain(`${workflowPath}:3:`);
    expect(result.stderr).toContain('step "broken"');
    expect(result.stderr).toContain('boom');
    expect(result.exitCode).toBe(1);
  });

  test('points a non-boolean branch test to its original test attribute', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-non-boolean.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      'needsReview: true',
      'needsReview: "yes"'
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML runtime error [WOML_BRANCH_TEST_NOT_BOOLEAN]'
    );
    expect(result.stderr).toContain(`${workflowPath}:19:`);
    expect(result.stderr).toContain('<when test> in branch "decision"');
    expect(result.stderr).toContain('must resolve to a JSON boolean');
    expect(result.exitCode).toBe(1);
  });

  test('points a missing condition reference to its original test attribute', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-missing-test.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      'needsReview: true',
      'otherProperty: true'
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML runtime error [WOML_REFERENCE_NOT_AVAILABLE]'
    );
    expect(result.stderr).toContain(`${workflowPath}:19:`);
    expect(result.stderr).toContain('<when test> in branch "decision"');
    expect(result.stderr).toContain('context.steps.checkContent.needsReview');
    expect(result.exitCode).toBe(1);
  });

  test('points a missing result reference to its original result attribute', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-missing-result.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      '{{context.steps.reviewContent}}',
      '{{context.steps.reviewContent.missing}}'
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML runtime error [WOML_REFERENCE_NOT_AVAILABLE]'
    );
    expect(result.stderr).toContain(`${workflowPath}:29:`);
    expect(result.stderr).toContain('<result value> in branch "decision"');
    expect(result.stderr).toContain('context.steps.reviewContent.missing');
    expect(result.exitCode).toBe(1);
  });

  test('runs from a clean package installation with its native Rust engine', async () => {
    const packageDirectory = join(temporaryDirectory, 'package');
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await mkdir(packageDirectory, { recursive: true });
    await mkdir(consumerDirectory, { recursive: true });
    await mkdir(bunTemporaryDirectory, { recursive: true });
    await mkdir(bunCacheDirectory, { recursive: true });
    await Bun.write(
      join(consumerDirectory, 'package.json'),
      JSON.stringify({ name: 'woml-clean-install-test', private: true })
    );
    await Bun.write(
      join(consumerDirectory, 'hello.woml'),
      await Bun.file(helloFixturePath).text()
    );
    await Bun.write(
      join(consumerDirectory, 'branch.woml'),
      await Bun.file(branchFixturePath).text()
    );
    await Bun.write(
      join(consumerDirectory, 'parallel.woml'),
      await Bun.file(parallelFixturePath).text()
    );
    await Bun.write(
      join(consumerDirectory, 'approval.woml'),
      await Bun.file(approvalFixturePath).text()
    );

    const packed = Bun.spawnSync(
      [
        Bun.which('bun')!,
        'pm',
        'pack',
        '--ignore-scripts',
        '--destination',
        packageDirectory,
      ],
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(packed.exitCode).toBe(0);
    const archive = (await readdir(packageDirectory))
      .filter(name => name.endsWith('.tgz'))
      .map(name => join(packageDirectory, name))[0];
    expect(archive).toBeDefined();

    const installed = Bun.spawnSync(
      [Bun.which('bun')!, 'add', archive!, '--no-save'],
      {
        cwd: consumerDirectory,
        env: {
          ...process.env,
          TMPDIR: bunTemporaryDirectory,
          BUN_INSTALL_CACHE_DIR: bunCacheDirectory,
        },
        stdout: 'pipe',
        stderr: 'pipe',
      }
    );
    if (installed.exitCode !== 0) {
      throw new Error(
        `Could not install packed WOML CLI:\n${installed.stdout.toString()}${installed.stderr.toString()}`
      );
    }
    const entriesBeforeRun = (await readdir(consumerDirectory)).sort();
    const executable = join(consumerDirectory, 'node_modules', '.bin', 'woml');
    const helloResult = Bun.spawnSync([executable, 'run', 'hello.woml'], {
      cwd: consumerDirectory,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const branchResult = Bun.spawnSync([executable, 'run', 'branch.woml'], {
      cwd: consumerDirectory,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const parallelResult = Bun.spawnSync([executable, 'run', 'parallel.woml'], {
      cwd: consumerDirectory,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const approval = spawnPackagedApproval(
      executable,
      [
        'run',
        'approval.woml',
        '--state',
        join(temporaryDirectory, 'packaged-approval.sqlite'),
        '--approval-port',
        String(await availablePort()),
      ],
      consumerDirectory
    );
    const approvalUrl = await approval.url;
    const approvalPage = await fetch(approvalUrl);
    const approvalResponse = await fetch(decisionEndpoint(approvalUrl), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ decision: 'approved' }),
    });
    const [approvalStdout, approvalStderr, approvalExitCode] =
      await Promise.all([
        approval.stdout,
        approval.stderr,
        approval.child.exited,
      ]);

    expect(helloResult.stdout.toString()).toBe('{"message":"Hello World"}\n');
    expect(helloResult.stderr.toString()).toBe('');
    expect(helloResult.exitCode).toBe(0);
    expect(branchResult.stdout.toString()).toBe(
      '{"message":"Final status: reviewed"}\n'
    );
    expect(branchResult.stderr.toString()).toBe('');
    expect(branchResult.exitCode).toBe(0);
    expect(parallelResult.stdout.toString()).toBe(
      '{"summary":"Weather 22°C, soil 41%"}\n'
    );
    expect(parallelResult.stderr.toString()).toBe('');
    expect(parallelResult.exitCode).toBe(0);
    expect(approvalPage.status).toBe(200);
    expect(await approvalPage.text()).toContain('Editorial approval');
    expect(approvalResponse.status).toBe(200);
    expect(JSON.parse(approvalStdout)).toEqual({
      decision: 'approved',
      source: 'human',
      published: true,
    });
    expect(approvalStderr).toContain('waiting for human approval');
    expect(approvalExitCode).toBe(0);
    expect((await readdir(consumerDirectory)).sort()).toEqual(entriesBeforeRun);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml-cli',
          'dist',
          `woml-core.${process.platform}-${process.arch}.node`
        )
      ).exists()
    ).toBe(true);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml-cli',
          'dist',
          'notification-provider-host.js'
        )
      ).exists()
    ).toBe(true);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml-cli',
          'slack',
          'manifest.json'
        )
      ).exists()
    ).toBe(true);
  });
});
