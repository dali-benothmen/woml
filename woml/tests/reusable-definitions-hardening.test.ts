import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  buildWomlReusableDefinitionPackage,
  parseWoml,
  resolveWomlReusableDefinitionGraph,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map(path =>
      rm(path, { recursive: true, force: true })
    )
  );
});

async function project(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), 'woml-reusable-hardening-'));
  temporaryDirectories.push(path);
  return path;
}

async function document(path: string) {
  return parseWoml(await readFile(path, 'utf8'), { file: path });
}

describe('reusable definition hardening', () => {
  test('many imports and repeated invocations produce one stable immutable package', async () => {
    const root = await project();
    const importTags: string[] = [];
    const invocations: string[] = [];
    for (let index = 0; index < 16; index += 1) {
      const alias = `operation-${index}`;
      const source = `${alias}.woml`;
      await writeFile(
        resolve(root, source),
        `<woml><props><prop name="value" required="true" /></props><step><script>return { value: Number(props.value) + ${index} };</script></step></woml>`
      );
      importTags.push(`<module name="${alias}" from="./${source}" />`);
      invocations.push(
        `<${alias} id="result${index}a" value="${index}" /><${alias} id="result${index}b" value="${index}" />`
      );
    }
    const workflowPath = resolve(root, 'workflow.woml');
    await writeFile(
      workflowPath,
      `<woml><imports>${importTags.join('')}</imports><workflow id="many-definitions"><triggers><manual id="start" /></triggers><steps>${invocations.join('')}<step id="done"><script>return { count: 32 };</script></step></steps></workflow></woml>`
    );
    const source = await document(workflowPath);
    const graph = resolveWomlReusableDefinitionGraph(source, {
      sourcePath: workflowPath,
      projectRoot: root,
    });
    const first = await buildWomlReusableDefinitionPackage(source, graph, {
      sourcePath: workflowPath,
      projectRoot: root,
    });
    const second = await buildWomlReusableDefinitionPackage(source, graph, {
      sourcePath: workflowPath,
      projectRoot: root,
    });
    expect(graph.definitions).toHaveLength(16);
    expect(first.workflow.model.reusableDefinitions).toHaveLength(32);
    expect(second.rootHash).toBe(first.rootHash);
    expect(new Set(first.artifacts.map(item => item.path)).size).toBe(
      first.artifacts.length
    );
  });

  test('duplicate canonical imports fail at compile time', async () => {
    const root = await project();
    await writeFile(
      resolve(root, 'shared.woml'),
      '<woml><step><script>return true;</script></step></woml>'
    );
    const workflowPath = resolve(root, 'workflow.woml');
    await writeFile(
      workflowPath,
      '<woml><imports><module name="first" from="./shared.woml" /><module name="second" from="./shared.woml" /></imports><workflow id="duplicate"><steps><first id="run" /></steps></workflow></woml>'
    );
    try {
      const source = await document(workflowPath);
      resolveWomlReusableDefinitionGraph(source, {
        sourcePath: workflowPath,
        projectRoot: root,
      });
      throw new Error('Expected duplicate canonical import rejection.');
    } catch (error) {
      expect(
        error instanceof WomlCompileError || error instanceof WomlValidationError
      ).toBe(true);
      expect((error as WomlCompileError | WomlValidationError).diagnostic.code).toBe(
        'WOML_MODULE_SOURCE_DUPLICATE'
      );
    }
  });

  test('missing and deleted definitions never fall back to stale current files', async () => {
    const root = await project();
    const missingWorkflow = resolve(root, 'missing.woml');
    await writeFile(
      missingWorkflow,
      '<woml><imports><module name="missing-step" from="./missing-step.woml" /></imports><workflow id="missing"><steps><missing-step id="run" /></steps></workflow></woml>'
    );
    try {
      const source = await document(missingWorkflow);
      resolveWomlReusableDefinitionGraph(source, {
        sourcePath: missingWorkflow,
        projectRoot: root,
      });
      throw new Error('Expected missing definition rejection.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlCompileError);
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_REUSABLE_SOURCE_NOT_FOUND'
      );
    }

    const definitionPath = resolve(root, 'available-step.woml');
    await writeFile(
      definitionPath,
      '<woml><step><script>return true;</script></step></woml>'
    );
    const workflowPath = resolve(root, 'changed.woml');
    await writeFile(
      workflowPath,
      '<woml><imports><module name="available-step" from="./available-step.woml" /></imports><workflow id="changed"><steps><available-step id="run" /></steps></workflow></woml>'
    );
    const source = await document(workflowPath);
    const graph = resolveWomlReusableDefinitionGraph(source, {
      sourcePath: workflowPath,
      projectRoot: root,
    });
    await unlink(definitionPath);
    await expect(
      buildWomlReusableDefinitionPackage(source, graph, {
        sourcePath: workflowPath,
        projectRoot: root,
      })
    ).rejects.toThrow();
  });
});
