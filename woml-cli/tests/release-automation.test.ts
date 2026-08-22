import { describe, expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { womlNativeTargets } from '../src/native-platform';

const repositoryRoot = resolve(import.meta.dir, '../..');

interface WorkflowJob {
  readonly name?: string;
  readonly if?: string;
  readonly environment?: string;
  readonly needs?: string | readonly string[];
  readonly permissions?: Readonly<Record<string, string>>;
  readonly strategy?: {
    readonly matrix?: { readonly include?: readonly Record<string, unknown>[] };
  };
  readonly steps?: ReadonlyArray<{
    readonly uses?: string;
    readonly with?: Readonly<Record<string, unknown>>;
  }>;
}

interface WorkflowDocument {
  readonly on?: Readonly<Record<string, unknown>>;
  readonly permissions?: Readonly<Record<string, string>>;
  readonly concurrency?: Readonly<Record<string, unknown>>;
  readonly jobs?: Readonly<Record<string, WorkflowJob>>;
}

async function workflow(name: string): Promise<{
  readonly source: string;
  readonly document: WorkflowDocument;
}> {
  const source = await readFile(
    resolve(repositoryRoot, `.github/workflows/${name}.yml`),
    'utf8',
  );
  return {
    source,
    document: Bun.YAML.parse(source) as WorkflowDocument,
  };
}

describe('WOML release automation', () => {
  test('keeps CI responsibilities visible and least-privileged', async () => {
    const { document, source } = await workflow('ci');
    expect(Object.keys(document.jobs ?? {}).sort()).toEqual([
      'architecture',
      'cli',
      'documentation',
      'frontend',
      'package',
      'rust',
    ]);
    expect(document.permissions).toEqual({ contents: 'read' });
    expect(document.concurrency).toMatchObject({ 'cancel-in-progress': true });
    expect(source).toContain('actions/checkout@v6');
    expect(source).toContain('actions/upload-artifact@v6');
    expect(source).not.toMatch(/actions\/(?:checkout|setup-node|upload-artifact|download-artifact)@v[1-5]\b/u);
  });

  test('builds every frozen native target on its matching runner', async () => {
    const { document } = await workflow('release');
    const matrix = document.jobs?.['build-native']?.strategy?.matrix?.include ?? [];
    const targets = matrix.map(entry => entry.package).sort();
    expect(targets).toEqual([...womlNativeTargets].sort());
    expect(new Set(matrix.map(entry => entry.rust)).size).toBe(6);
    expect(matrix.every(entry => typeof entry.runner === 'string')).toBe(true);
  });

  test('clean-installs and executes every platform candidate before collection', async () => {
    const { document, source } = await workflow('release');
    const smoke = await readFile(
      resolve(repositoryRoot, 'woml-cli/scripts/smoke-release-candidate.ts'),
      'utf8',
    );
    expect(document.jobs?.['build-native']?.needs).toEqual([
      'validate',
      'build-main',
    ]);
    expect(source).toContain('name: woml-main');
    expect(source.match(/scripts\/smoke-release-candidate\.ts/gu)).toHaveLength(1);
    for (const marker of [
      "command('--version')",
      "command('--help')",
      "command('check', 'hello.woml')",
      "command('check', 'for-each.woml')",
      "join(directory, 'for-each-state.sqlite')",
      'context.iteration.index',
      "'test',",
      'install();',
      'The portable woml package contains a native binary',
    ]) {
      expect(smoke).toContain(marker);
    }
  });

  test('makes tags non-publishing and requires an approved exact-tag dispatch', async () => {
    const { document, source } = await workflow('release');
    const publish = document.jobs?.publish;
    expect(document.on).toHaveProperty('push');
    expect(document.on).toHaveProperty('workflow_dispatch');
    expect(publish?.environment).toBe('npm-production');
    expect(String(publish?.if)).toContain("github.event_name == 'workflow_dispatch'");
    expect(String(publish?.if)).toContain('inputs.publish_to_npm == true');
    expect(String(publish?.if)).toContain("github.ref_type == 'tag'");
    expect(String(publish?.if)).toContain(
      "vars.WOML_NPM_PUBLISH_ENABLED == 'true'",
    );
    expect(publish?.permissions).toEqual({
      contents: 'write',
      'id-token': 'write',
    });
    expect(source).not.toContain('NPM_TOKEN');
    expect(source).not.toContain('NODE_AUTH_TOKEN');
    expect(source).toContain('bun run test:for-each-contracts');
    expect(source).toContain('npm publish "${archive}" --access public --provenance');
    expect(source).toContain('bun run test:final-review');
    expect(source).toContain('bun audit --cwd woml-cli');
    expect(source).toContain('cargo-audit --version 0.22.2 --locked');
    expect(source).toContain('cargo audit --file core/Cargo.lock --ignore RUSTSEC-2026-0258');
  });

  test('publishes only the immutable family that the collection job verified', async () => {
    const { document, source } = await workflow('release');
    const artifactSource = await readFile(
      resolve(repositoryRoot, 'woml-cli/scripts/release-artifact.ts'),
      'utf8',
    );
    expect(document.jobs?.collect?.needs).toEqual([
      'validate',
      'build-main',
      'build-native',
    ]);
    expect(document.jobs?.publish?.needs).toEqual(['validate', 'collect']);
    expect(source).toContain('name: woml-release-family');
    expect(`${source}\n${artifactSource}`).toContain('artifact-sha256.json');
    expect(`${source}\n${artifactSource}`).toContain('native-load-test.json');
    expect(source).toContain('Create the GitHub release after npm succeeds');
    expect(source).toContain('tar -czf woml-skill.tar.gz -C skills/woml .');
    expect(source).toContain(
      'gh release upload "${GITHUB_REF_NAME}" woml-skill.tar.gz --clobber',
    );
    expect(source.indexOf('Publish woml last')).toBeLessThan(
      source.indexOf('Create the GitHub release after npm succeeds'),
    );
  });
});
