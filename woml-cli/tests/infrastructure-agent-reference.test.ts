import { describe, expect, test } from 'bun:test';
import { readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { resolveWomlProject } from '../src/project';

const root = resolve(import.meta.dir, '../../examples/infrastructureAgent');
const manifest = resolve(root, 'workflow.json');

function output() {
  const stdout: string[] = [];
  const stderr: string[] = [];
  const io: CliIo = {
    stdout: value => stdout.push(value),
    stderr: value => stderr.push(value),
  };
  return { io, stdout, stderr };
}

describe('infrastructure-agent reference project', () => {
  test('resolves every reviewed authority while worker secrets stay outside the manifest', async () => {
    const project = await resolveWomlProject(manifest, {
      mode: 'background',
      environment: {},
    });
    expect(project.workflows.flatMap(item => item.workflowIds)).toEqual([
      'infrastructure-agent-deployment',
    ]);
    expect(project.authorities.process).toBeDefined();
    expect(project.authorities.artifacts).toBeDefined();
    expect(project.authorities.infrastructure).toBeDefined();
    expect(project.authorities.reviewers).toBeDefined();
    expect(project.authorities.targets).toBeDefined();
    expect(JSON.stringify(project.manifest)).not.toContain('privateKey');
    expect(JSON.stringify(project.manifest)).not.toContain('credential');
  });

  test('preflights all ten composed capabilities without starting external systems', async () => {
    const captured = output();
    expect(
      await runCli(['check', '--project', manifest, '--json'], captured.io)
    ).toBe(0);
    const report = JSON.parse(captured.stdout.join('')) as {
      status: string;
      subsystems: readonly { id: string; status: string }[];
    };
    expect(report.status).toBe('degraded');
    expect(Object.fromEntries(report.subsystems.map(item => [item.id, item.status])))
      .toMatchObject({
        process: 'ready',
        artifacts: 'ready',
        approvals: 'ready',
        'infrastructure-policy': 'ready',
        'remote-execution': 'ready',
        'external-signals': 'ready',
        'streaming-output': 'ready',
        opentelemetry: 'degraded',
      });
    expect(captured.stderr.join('')).toBe('');
  });

  test('ships executable deterministic adapters and a non-authoritative worker template', async () => {
    for (const adapter of [
      'terraform.ts',
      'prometheus.ts',
      'ai-review.ts',
      'slack.ts',
    ]) {
      const metadata = await stat(resolve(root, 'adapters', adapter));
      expect(metadata.isFile()).toBe(true);
      if (process.platform !== 'win32') expect(metadata.mode & 0o111).not.toBe(0);
    }
    const worker = await Bun.file(
      resolve(root, 'operator/woml.worker.json')
    ).json();
    expect(worker).toMatchObject({
      workerId: 'rehearsal-worker-01',
      targetIds: ['production-private'],
    });
    expect(worker.tls.privateKeyRef).toContain('private/');
    expect(
      await Bun.file(
        resolve(root, 'operator/private/rehearsal-worker-01.key')
      ).exists()
    ).toBe(false);
  });

  test('fake integrations execute deterministically without contacting external systems', async () => {
    const cases = [
      ['terraform.ts', 'apply', 'release-rehearsal-42'],
      ['terraform.ts', 'rollback', 'restored'],
      ['prometheus.ts', 'health', '99.99'],
      ['ai-review.ts', 'classify', 'deterministic-rehearsal-v1'],
      ['slack.ts', 'notify', 'fake-slack'],
    ] as const;
    for (const [adapter, command, expected] of cases) {
      const result = Bun.spawnSync({
        cmd: [process.execPath, resolve(root, 'adapters', adapter), command],
        env: {},
        stdout: 'pipe',
        stderr: 'pipe',
      });
      expect(result.exitCode).toBe(0);
      expect(result.stderr.toString()).toBe('');
      expect(result.stdout.toString()).toContain(expected);
    }
  });

  test('ships a loopback-only collector with a local debug exporter', async () => {
    const collector = await readFile(
      resolve(root, 'collector/otel-collector.yaml'),
      'utf8'
    );
    expect(collector).toContain('endpoint: 127.0.0.1:4318');
    expect(collector).toContain('debug:');
    expect(collector).not.toContain('otlphttp/');
  });
});
