#!/usr/bin/env bun
// ISR4 verification only. Uses retained artifacts; no native/installed release gate.
import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, platform } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { auditPrototypeReceipt } from './invocation-setup-review';
import { sha256 } from './invocation-setup-fixtures';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';

const root = resolve(import.meta.dir, '../..'), cli = join(root, 'woml-cli');
const harness = ['woml-cli/scripts/verify-invocation-setup.ts', 'woml-cli/scripts/invocation-setup-review.ts',
  'woml-cli/tests/script-host.test.ts', 'core/woml-engine/tests/script_host_protocol.rs',
  'core/woml-engine/tests/services_script_host_v4.rs', 'core/woml-engine/tests/workflow_call_composition.rs',
  'core/woml-engine/tests/cancellation_runtime.rs'];
async function current() {
  return { source: await snapshot(), harness: await Promise.all(harness.map(async path => ({ path, sha256: sha256(await readFile(join(root, path))) }))) };
}
if (import.meta.main) {
  const [flag, path] = process.argv.slice(2); assert.ok(flag === '--receipt' && path && process.argv.length === 4, 'Use --receipt RETAINED_PROTOTYPE_BUILD_RECEIPT');
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'RUST_MIN_STACK', 'NODE_OPTIONS', 'BUN_OPTIONS', 'WOML_SCRIPT_HOST_TEST_ENTRY', 'WOML_SCRIPT_HOST_TEST_WORKER']) assert.ok(!process.env[name], `Unset ${name}`);
  const receipt = JSON.parse(await readFile(resolve(path), 'utf8')); await auditPrototypeReceipt(receipt);
  assert.equal(Bun.version, receipt.bun); assert.equal(sha256(await readFile(process.execPath)), receipt.bunExecutableSha256);
  const rustBun = Bun.which('bun'); assert.ok(rustBun, 'Rust tests must not skip for missing Bun on PATH');
  assert.equal(sha256(await readFile(rustBun)), receipt.bunExecutableSha256, 'Rust tests must use the qualified Bun executable');
  await headroom(); const before = await current();
  const production = (files: { path: string; sha256: string }[]) => files.filter(file => !file.path.includes('/tests/')
    && (file.path.startsWith('core/') || file.path.startsWith('woml/src/') || file.path.startsWith('woml-cli/src/')));
  assert.deepEqual(production(before.source.files), production(receipt.inputs.source.files), 'Production differs from retained candidate/reference checkpoint');
  const parent = join(cli, '.woml/performance'); await mkdir(parent, { recursive: true });
  const output = await mkdtemp(join(parent, 'invocation-setup-verification-'));
  for (const file of before.harness) {
    const target = join(output, 'harness', file.path); await mkdir(dirname(target), { recursive: true });
    await copyFile(join(root, file.path), target);
    assert.equal(sha256(await readFile(target)), file.sha256);
  }
  const runs: any[] = [];
  await Bun.write(join(output, 'inputs.json'), JSON.stringify({ receipt, current: before, bun: Bun.version, platform: platform(), arch: arch(), releaseAccepted: false }, null, 2));
  const cargo = ['cargo', 'test', '--offline', '--config', 'profile.dev.debug=0', '--config', 'profile.dev.incremental=false', '-j', '1', '--locked', '--manifest-path', 'core/Cargo.toml', '-p', 'woml-engine'];
  async function run(name: string, argv: string[], cwd: string, extraEnv: Record<string, string> = {}) {
    assert.deepEqual(await current(), before); await auditPrototypeReceipt(receipt); const available = await headroom();
    const started = performance.now(), child = Bun.spawn(argv, { cwd, env: { ...process.env, ...extraEnv }, stdout: 'pipe', stderr: 'pipe', timeout: 900_000 });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
    const log = `${await stdout}\n${await stderr}`; await Bun.write(join(output, `${name}.log`), log);
    const counts = [...log.matchAll(/(?:^|\n)\s*(\d+) pass\b|test result: ok\. (\d+) passed/g)].map(match => Number(match[1] ?? match[2]));
    runs.push({ name, argv, environment: extraEnv, exitCode, testCounts: counts, elapsedMs: performance.now() - started,
      headroom: available, usage: resourceRecord(child.resourceUsage()), logSha256: sha256(log) });
    await Bun.write(join(output, 'verification.json'), JSON.stringify({ completed: false, runs, releaseAccepted: false, integrationAccepted: false }, null, 2));
    assert.equal(exitCode, 0, `Verification failed; retained ${output}/${name}.log\n${log.slice(-3000)}`);
    assert.ok(counts.some(count => count > 0), 'A selected suite ran zero tests');
    assert.deepEqual(await current(), before); console.log(`${name}: passed (${counts.join(' + ')} tests)`);
  }
  await run('rust-settlement', [...cargo, '--lib', 'host::tests', '--', '--test-threads=1'], root);
  await run('rust-authority', [...cargo, '--test', 'capability_authority', '--', '--test-threads=1'], root);
  for (const variant of ['reference', 'candidate']) {
    const find = (name: string) => receipt.artifacts.find((file: any) => file.variant === variant && file.name === name).path;
    const env = { WOML_SCRIPT_HOST_TEST_ENTRY: find('script-host.js'), WOML_SCRIPT_HOST_TEST_WORKER: find('script-host-worker.js') };
    await run(`${variant}-bun`, [process.execPath, 'test', 'tests/script-host.test.ts', '--max-concurrency=1'], cli, env);
    await run(`${variant}-rust`, [...cargo, '--test', 'script_host_protocol', '--test', 'services_script_host_v4',
      '--test', 'workflow_call_composition', '--test', 'cancellation_runtime', '--', '--test-threads=1'], root, env);
  }
  await auditPrototypeReceipt(receipt);
  await Bun.write(join(output, 'verification.json'), JSON.stringify({ completed: true, runs, releaseAccepted: false, integrationAccepted: false }, null, 2));
  console.log(`ISR4 verification: ${output}`);
}
