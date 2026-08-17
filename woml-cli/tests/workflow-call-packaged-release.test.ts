import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { nativePackageBinaryName } from '../src/native-platform';
import { installLocalReleaseCandidate } from './helpers/release-candidate';

const projectRoot = resolve(import.meta.dir, '../..');
const exampleDirectory = join(projectRoot, 'examples', 'workflowCalls');
let temporaryDirectory: string;

interface CapturedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stderr: () => string;
  readonly stderrDone: Promise<void>;
}

function start(executable: string, args: readonly string[], cwd: string): CapturedProcess {
  const child = Bun.spawn([executable, ...args], {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  let stderr = '';
  const stderrDone = (async () => {
    const reader = child.stderr.getReader();
    const decoder = new TextDecoder();
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      stderr += decoder.decode(chunk.value, { stream: true });
    }
    stderr += decoder.decode();
  })();
  return { child, stderr: () => stderr, stderrDone };
}

async function waitFor(process: CapturedProcess, text: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  while (!process.stderr().includes(text)) {
    if (process.child.exitCode !== null) {
      await process.stderrDone;
      throw new Error(
        `Packed WOML process exited before ${JSON.stringify(text)}:\n${process.stderr()}`,
      );
    }
    if (Date.now() >= deadline) throw new Error(process.stderr());
    await Bun.sleep(10);
  }
}

async function freePort(): Promise<number> {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response('reserved'),
  });
  const port = server.port!;
  await server.stop(true);
  return port;
}

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-wc7-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Packaged Workflow Calls release journey', () => {
  test('a clean consumer runs a parent and child with the packaged native engine', async () => {
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [consumerDirectory, bunTemporaryDirectory, bunCacheDirectory].map(
        directory => mkdir(directory, { recursive: true })
      )
    );
    await Bun.write(
      join(consumerDirectory, 'package.json'),
      JSON.stringify({ name: 'woml-wc7-clean-consumer', private: true })
    );
    await Bun.write(
      join(consumerDirectory, 'parent.woml'),
      `<woml>
  <workflow id="request-risk" name="Request customer risk" version="1.0.0">
    <triggers>
      <webhook id="request" path="/risk" method="POST" auth="none" />
    </triggers>
    <steps>
      <step id="requestRisk"><script>
        const risk = await services.workflows.call('calculate-risk', {
          customerId: context.payload.customerId
        });
        return {
          message: \`Customer risk score: \${risk.score}\`,
          score: risk.score
        };
      </script></step>
    </steps>
  </workflow>
</woml>`,
    );
    await Bun.write(
      join(consumerDirectory, 'child.woml'),
      await Bun.file(join(exampleDirectory, 'calculate-risk.woml')).text()
    );

    const candidate = await installLocalReleaseCandidate(consumerDirectory, {
      cache: bunCacheDirectory,
      temporary: bunTemporaryDirectory,
    });

    const executable = join(consumerDirectory, 'node_modules', '.bin', 'woml');
    const port = await freePort();
    const runtime = start(
      executable,
      [
        'run',
        'parent.woml',
        'child.woml',
        '--port',
        String(port),
        '--state',
        join(temporaryDirectory, 'packaged-state.sqlite'),
      ],
      consumerDirectory,
    );
    await waitFor(runtime, 'WOML automation is active. Press Ctrl+C to stop.');
    const response = await fetch(`http://127.0.0.1:${port}/risk`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ customerId: 'customer-42' }),
    });
    expect(response.status).toBe(202);
    await waitFor(runtime, 'Customer risk score: 90');
    const runtimeLog = runtime.stderr();
    runtime.child.kill('SIGINT');
    expect(await runtime.child.exited).toBe(0);
    await runtime.stderrDone;
    expect(runtimeLog).toContain('WOML automation is active. Press Ctrl+C to stop.');
    expect(runtimeLog).toContain('Workflow call · calculate-risk completed');
    expect(runtimeLog).toContain('run_call_');
    expect(runtimeLog).not.toContain('customer-42');

    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          ...candidate.nativePackage.split('/'),
          nativePackageBinaryName(candidate.target),
        )
      ).exists()
    ).toBe(true);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml',
          'dist',
          'script-host.js'
        )
      ).exists()
    ).toBe(true);
  }, 45_000);
});
