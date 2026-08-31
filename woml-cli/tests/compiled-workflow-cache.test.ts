import { describe, expect, test } from 'bun:test';
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

import {
  compiledWorkflowCachePath,
  readCompiledWorkflowCache,
  writeCompiledWorkflowCache,
  type CompiledWorkflowCacheArtifact,
  type CompiledWorkflowCacheOptions,
} from '../src/compiled-workflow-cache';
import { compileWorkflowInputs } from '../src/cli';
import { compiledDefinitionHash } from '../src/rust-executor';

function digest(value: string): string {
  return `sha256:${new Bun.CryptoHasher('sha256')
    .update(value)
    .digest('hex')}`;
}

function simpleWorkflow(id = 'cached-workflow'): string {
  return `<woml>
  <workflow id="${id}" name="Cached workflow" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="done"><script>return { ok: true };</script></step></steps>
  </workflow>
</woml>`;
}

function cacheFixture(root: string): {
  readonly sourcePath: string;
  readonly options: CompiledWorkflowCacheOptions;
  readonly artifact: CompiledWorkflowCacheArtifact;
} {
  const sourcePath = join(root, 'workflow.woml');
  const source = simpleWorkflow();
  writeFileSync(sourcePath, source);
  const workflow = compileWoml(parseWoml(source, { file: sourcePath }));
  return {
    sourcePath,
    options: {
      sourcePath,
      projectRoot: root,
      compilerIdentity: 'test-compiler-v1',
    },
    artifact: {
      workflow,
      definitionHash: compiledDefinitionHash(workflow),
      runtimeModules: [],
      sourceSnapshot: [{ path: sourcePath, digest: digest(source) }],
      migrationDiagnostics: [],
    },
  };
}

describe('compiled workflow cache', () => {
  test('round-trips a validated artifact through an atomic private cache file', async () => {
    const root = mkdtempSync(join(tmpdir(), 'woml-compiled-cache-'));
    try {
      const fixture = cacheFixture(root);
      expect(
        await writeCompiledWorkflowCache(fixture.options, fixture.artifact)
      ).toBe(true);
      expect(await readCompiledWorkflowCache(fixture.options)).toEqual(
        fixture.artifact
      );
      if (process.platform !== 'win32') {
        expect(
          statSync(compiledWorkflowCachePath(fixture.options)).mode & 0o777
        ).toBe(0o600);
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('turns source, compiler, and corrupted-artifact changes into safe misses', async () => {
    const root = mkdtempSync(join(tmpdir(), 'woml-compiled-cache-'));
    try {
      const fixture = cacheFixture(root);
      expect(
        await writeCompiledWorkflowCache(fixture.options, fixture.artifact)
      ).toBe(true);

      writeFileSync(fixture.sourcePath, `${simpleWorkflow()}\n`);
      expect(await readCompiledWorkflowCache(fixture.options)).toBeUndefined();
      writeFileSync(fixture.sourcePath, simpleWorkflow());
      expect(
        await readCompiledWorkflowCache({
          ...fixture.options,
          compilerIdentity: 'different-compiler',
        })
      ).toBeUndefined();

      const path = compiledWorkflowCachePath(fixture.options);
      const envelope = JSON.parse(readFileSync(path, 'utf8'));
      envelope.artifact.workflow.workflowId = 'tampered';
      writeFileSync(path, JSON.stringify(envelope));
      expect(await readCompiledWorkflowCache(fixture.options)).toBeUndefined();
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('reuses module artifacts and invalidates them when imported code changes', async () => {
    const root = mkdtempSync(join(tmpdir(), 'woml-compiled-cache-module-'));
    try {
      const sourcePath = join(root, 'workflow.woml');
      const modulePath = join(root, 'math.ts');
      writeFileSync(join(root, 'package.json'), '{"private":true}\n');
      writeFileSync(
        sourcePath,
        `<woml>
  <imports><module name="math" from="./math.ts" /></imports>
  <workflow id="module-cache" name="Module cache" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="calculate"><script>return { value: services.math.value() };</script></step></steps>
  </workflow>
</woml>`
      );
      writeFileSync(modulePath, 'export function value() { return 1; }\n');

      const first = await compileWorkflowInputs([sourcePath]);
      expect(first[0]?.runtimeModules).toHaveLength(1);
      const cacheFiles = join(root, '.woml', 'cache', 'compiled-v1');
      const cachePath = join(
        cacheFiles,
        `${new Bun.CryptoHasher('sha256').update(sourcePath).digest('hex')}.json`
      );
      const firstCache = readFileSync(cachePath, 'utf8');

      const second = await compileWorkflowInputs([sourcePath]);
      expect(second[0]?.definitionHash).toBe(first[0]?.definitionHash);
      expect(second[0]?.runtimeModules).toEqual(first[0]?.runtimeModules);
      expect(readFileSync(cachePath, 'utf8')).toBe(firstCache);

      writeFileSync(modulePath, 'export function value() { return 2; }\n');
      const changed = await compileWorkflowInputs([sourcePath]);
      expect(changed[0]?.definitionHash).not.toBe(first[0]?.definitionHash);
      expect(changed[0]?.runtimeModules[0]?.bundleDigest).not.toBe(
        first[0]?.runtimeModules[0]?.bundleDigest
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('caches imported WOML definitions and invalidates their lowered model', async () => {
    const root = mkdtempSync(join(tmpdir(), 'woml-compiled-cache-reusable-'));
    try {
      const sourcePath = join(root, 'workflow.woml');
      const reusablePath = join(root, 'prepare.woml');
      writeFileSync(join(root, 'package.json'), '{"private":true}\n');
      writeFileSync(
        sourcePath,
        `<woml>
  <imports><module name="prepare" from="./prepare.woml" /></imports>
  <workflow id="reusable-cache" name="Reusable cache" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><prepare id="prepared" value="hello" /></steps>
  </workflow>
</woml>`
      );
      const reusable = (changed: boolean): string => `<woml>
  <props><prop name="value" required="true" /></props>
  <step name="Prepare"><script>return { value: props.value, changed: ${changed} };</script></step>
</woml>`;
      writeFileSync(reusablePath, reusable(false));

      const first = await compileWorkflowInputs([sourcePath]);
      expect(first[0]?.runtimeModules.length).toBeGreaterThan(0);
      const cachePath = compiledWorkflowCachePath({
        sourcePath,
        projectRoot: root,
        compilerIdentity: 'path-only',
      });
      const cachedEnvelope = JSON.parse(readFileSync(cachePath, 'utf8'));
      expect(cachedEnvelope.artifact.reusableEditorData).toContain('prepare');
      const second = await compileWorkflowInputs([sourcePath]);
      expect(second[0]?.definitionHash).toBe(first[0]?.definitionHash);

      writeFileSync(reusablePath, reusable(true));
      const changed = await compileWorkflowInputs([sourcePath]);
      expect(changed[0]?.definitionHash).not.toBe(first[0]?.definitionHash);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
