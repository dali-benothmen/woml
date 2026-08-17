import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import {
  buildWomlReusableDefinitionPackage,
  parseWoml,
  resolveWomlReusableDefinitionGraph,
} from '../src';

const fixtureRoot = resolve(import.meta.dir, 'fixtures/reusable-definitions');
const workflowPath = resolve(fixtureRoot, 'mixed-provider-workflow.woml');

describe('cross-provider authoring composition', () => {
  test('lowers built-in and custom providers through approval, lifecycle, and nested control flow', async () => {
    const document = parseWoml(readFileSync(workflowPath, 'utf8'), {
      file: workflowPath,
    });
    const graph = resolveWomlReusableDefinitionGraph(document, {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const definitionPackage = await buildWomlReusableDefinitionPackage(
      document,
      graph,
      { sourcePath: workflowPath, projectRoot: fixtureRoot }
    );

    expect(definitionPackage.runtimeReady).toBe(true);
    expect(definitionPackage.workflow.model.schemaVersion).toBe(15);
    expect(definitionPackage.definitions).toContainEqual(
      expect.objectContaining({
        alias: 'local-approval',
        kind: 'notification-provider',
      })
    );
    expect(definitionPackage.workflow.model.reusableDefinitions).toContainEqual(
      expect.objectContaining({
        kind: 'notification-provider',
        alias: 'local-approval',
      })
    );

    const serialized = JSON.stringify(definitionPackage.workflow.model);
    for (const provider of [
      'slack',
      'telegram',
      'discord',
      'whatsapp',
      'custom',
    ]) {
      expect(serialized).toContain(
        `\"provider\":{\"kind\":\"literal\",\"value\":\"${provider}\"}`
      );
    }
    expect(serialized).toContain('engine.choice-select');
    expect(serialized).toContain('engine.fork-open');
    expect(serialized).toContain('engine.approval-wait');
    expect(serialized).not.toContain('xoxb-');
    expect(serialized).not.toContain('ncap_');
  });
});
