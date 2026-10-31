import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { verifyRetryExampleContract } from '../scripts/retry-example-contract';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const [example, fixture, frozenModel] = await Promise.all([
  Bun.file(resolve(projectRoot, 'examples/retryWorkflow.woml')).text(),
  Bun.file(resolve(projectRoot, 'woml/tests/fixtures/retry.woml')).text(),
  Bun.file(resolve(projectRoot, 'woml/tests/fixtures/retry.compiled.v6.json')).json(),
]);

describe('Retry release example compatibility', () => {
  test('preserves the historical Model v6 while reviewing only two public-source edits', () => {
    const compiled = verifyRetryExampleContract(example, fixture, frozenModel);
    expect(compiled.schemaVersion).toBe(6);
    expect(compiled.metadata?.version).toBe('1.0.0');
    expect(frozenModel.metadata.version).toBe('0.1');
    expect(compiled.graph.nodes.map(node => node.retryPolicy)).toEqual(
      frozenModel.graph.nodes.map((node: { retryPolicy?: unknown }) => node.retryPolicy),
    );
    expect(compiled.graph.nodes[1]?.retryPolicy).toEqual({
      maxAttempts: 3,
      backoff: { kind: 'exponential', initialDelayMs: 1000, multiplier: 2, maximumDelayMs: 30000 },
    });
  });

  test('allows platform line endings without changing the historical script bytes in the model', () => {
    expect(verifyRetryExampleContract(example.replace(/\r?\n/gu, '\r\n'), fixture, frozenModel))
      .toEqual(verifyRetryExampleContract(example, fixture.replace(/\r?\n/gu, '\r\n'), frozenModel));
  });

  for (const [label, before, after] of [
    ['attempt count', 'retry="3"', 'retry="4"'],
    ['backoff strategy', 'retry-backoff="exponential"', 'retry-backoff="fixed"'],
    ['initial delay', 'retry-delay="1s"', 'retry-delay="2s"'],
    ['delay cap', 'retry-max-delay="30s"', 'retry-max-delay="60s"'],
    ['retry behavior', 'attempt.number < 3', 'attempt.number < 2'],
    ['failure script', 'Temporary greeting failure', 'Different failure'],
    ['result script', 'Hello ${context.steps.prepare.name}', 'Goodbye ${context.steps.prepare.name}'],
    ['workflow identity', 'id="retry-demo"', 'id="another-workflow"'],
    ['step identity', 'id="greet"', 'id="another-step"'],
    ['metadata', 'name="Retry Demo"', 'name="Changed Demo"'],
    ['trigger', '<manual id="start" />', '<manual id="other" />'],
    ['payload field', 'context.payload.name', 'context.payload.customer'],
    ['historical alias in the public example', 'context.payload.name', 'context.trigger.name'],
    ['unreviewed public version', 'version="1.0.0"', 'version="2.0.0"'],
    ['extra graph node', '</steps>', '<step id="extra"><script>return 1;</script></step></steps>'],
  ] as const) {
    test(`rejects unreviewed changes to ${label}`, () => {
      expect(example).toContain(before);
      expect(() => verifyRetryExampleContract(example.replace(before, after), fixture, frozenModel))
        .toThrow('the public retry example may differ only');
    });
  }

  test('rejects missing or duplicated migration anchors even when compilation is unchanged', () => {
    for (const source of [
      fixture.replace('version="0.1"', "version='0.1'"),
      fixture.replace('<woml>', '<woml><!-- version="0.1" -->'),
    ]) {
      expect(() => verifyRetryExampleContract(example, source, frozenModel))
        .toThrow('reviewed retry migration requires exactly one');
    }
  });

  test('rejects coordinated source drift rather than accepting two equally changed examples', () => {
    expect(() => verifyRetryExampleContract(
      example.replace('retry="3"', 'retry="4"'),
      fixture.replace('retry="3"', 'retry="4"'),
      frozenModel,
    )).toThrow('historical retry source must compile to its frozen Model v6');
  });

  test('rejects frozen-model drift rather than regenerating historical expectations', () => {
    expect(() => verifyRetryExampleContract(example, fixture, { ...frozenModel, schemaVersion: 7 }))
      .toThrow('historical retry source must compile to its frozen Model v6');
  });

  // Separate processes isolate Bun's module mocks from all other test files.
  // Only N6's heavyweight shared build/test suite is replaced. Every actual
  // source verifier on the T13 -> RI7 import chain still runs unmodified, and
  // reaching N6 is mandatory. This is NOT a full release/native certification.
  for (const [entry, marker] of [
    ['verify-retry-release.ts', '[RI7] retry release gate passed'],
    ['verify-production-triggers-release.ts', '[T13] complete Production Triggers release gate passed'],
  ] as const) {
    test(`${entry} reaches the shared suite through the real source-contract chain`, async () => {
      const script = `
        import { mock } from 'bun:test';
        let sharedSuiteReached = false;
        mock.module(${JSON.stringify(pathToFileURL(resolve(cliRoot, 'scripts/verify-slack-notification-release.ts')).href)}, () => {
          sharedSuiteReached = true;
          return {};
        });
        await import(${JSON.stringify(pathToFileURL(resolve(cliRoot, 'scripts', entry)).href)});
        if (!sharedSuiteReached) throw new Error('Shared N6 suite was disconnected from the release gate.');
        console.log('[test-only] shared N6 native/build suite stubbed; source checks complete');
      `;
      const child = Bun.spawn([process.execPath, '--eval', script], {
        cwd: cliRoot, stdout: 'pipe', stderr: 'pipe',
      });
      const watchdog = setTimeout(() => {
        if (child.exitCode === null) child.kill();
      }, 10_000);
      try {
        const [stdout, stderr, exitCode] = await Promise.all([
          new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ]);
        expect({ exitCode, stderr }).toEqual({ exitCode: 0, stderr: '' });
        expect(stdout).toContain('[RI7] retry example matches the reviewed public migration');
        expect(stdout).toContain(marker);
        expect(stdout).toContain('[test-only] shared N6 native/build suite stubbed; source checks complete');
      } finally {
        clearTimeout(watchdog);
        if (child.exitCode === null) {
          child.kill();
          await child.exited;
        }
      }
    }, 15_000);
  }
});
