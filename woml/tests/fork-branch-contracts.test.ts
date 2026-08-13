import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  compileWoml,
  inspectWomlMigrationDiagnostics,
  parseWoml,
  WomlValidationError,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/fork-branch');
const sharedFixtureRoot = resolve(import.meta.dir, 'fixtures');

function fixture(name: string): string {
  return readFileSync(resolve(fixtureRoot, name), 'utf8');
}

function sharedFixture(name: string): string {
  return readFileSync(resolve(sharedFixtureRoot, name), 'utf8');
}

function jsonFixture(name: string): any {
  return JSON.parse(fixture(name));
}

function contractValidators(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat(
    'date-time',
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/
  );
  const names = [
    ...Array.from(
      { length: 13 },
      (_, index) => `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 12 },
      (_, index) => `run-event.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 8 },
      (_, index) => `woml-definition-package.v${index + 1}.schema.json`
    ),
    'runtime-policy.v1.schema.json',
    'woml-template.v1.schema.json',
    'run-inspection.v2.schema.json',
    'run-inspection.v3.schema.json',
    'run-inspection.v4.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema(JSON.parse(readFileSync(resolve(schemaRoot, name), 'utf8')));
  }
  return ajv;
}

describe('FJ0 frozen fork and branch contracts', () => {
  test('validates Model v13, Event v12, Definition Package v8, and Run Inspection v4 artifacts', () => {
    const ajv = contractValidators();
    const model = jsonFixture('join-all.compiled.v13.json');
    const contracts = jsonFixture('contracts.v1.json');
    const histories = jsonFixture('histories.v12.json');

    const validateModel = ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v13'
    )!;
    expect(
      validateModel(model),
      JSON.stringify(validateModel.errors, null, 2)
    ).toBe(true);

    const validateEvent = ajv.getSchema(
      'https://cronflow.dev/schemas/run-event/v12'
    )!;
    for (const history of Object.values(histories) as readonly any[][]) {
      for (const event of history) {
        expect(
          validateEvent(event),
          JSON.stringify(validateEvent.errors, null, 2)
        ).toBe(true);
      }
    }

    const hash = `sha256:${'0'.repeat(64)}`;
    const definitionPackage = {
      schemaVersion: 8,
      profile: 'woml.definition-package/v8',
      executable: true,
      runtimeReady: false,
      workflow: {
        id: model.workflowId,
        source: 'join-all.woml',
        modelDigest: hash,
        model,
      },
      modules: [],
      sources: [
        {
          path: 'join-all.woml',
          mediaType: 'application/woml+xml',
          digest: hash,
          dependencies: [],
        },
      ],
      artifacts: [
        {
          path: 'join-all.compiled.v13.json',
          kind: 'workflow-model',
          mediaType: 'application/json',
          digest: hash,
          content: '{}',
        },
      ],
      compiler: {
        name: 'woml',
        version: '0.1.0',
        resolverProfile: 'woml.module-resolver/v1',
        bundler: {
          name: 'bun',
          version: '1.3.14',
          target: 'bun',
          format: 'esm',
          sourceMap: 'external',
        },
      },
      permissions: { secrets: [], networkOrigins: [] },
      rootHash: hash,
    };
    const validatePackage = ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v8.schema.json'
    )!;
    expect(
      validatePackage(definitionPackage),
      JSON.stringify(validatePackage.errors, null, 2)
    ).toBe(true);

    const validateInspection = ajv.getSchema(
      'https://woml.dev/schemas/run-inspection/v4'
    )!;
    expect(
      validateInspection(contracts.inspection),
      JSON.stringify(validateInspection.errors, null, 2)
    ).toBe(true);
  });

  test('freezes selected-join release independently from unjoined failure timing', () => {
    const histories = jsonFixture('histories.v12.json');
    for (const name of [
      'unjoinedFailureBeforeRelease',
      'unjoinedFailureAfterRelease',
    ]) {
      const types = histories[name].map((event: any) => event.type);
      expect(types).toContain('fork_join_settled');
      expect(
        histories[name].find((event: any) => event.type === 'fork_join_settled')
          .data.outcome
      ).toBe('succeeded');
      expect(
        histories[name].find(
          (event: any) =>
            event.type === 'fork_branch_settled' &&
            event.data.branchId === 'analytics'
        ).data.outcome
      ).toBe('failed');
    }
  });

  test('freezes deterministic visibility, failure results, lifecycle ordering, recovery, and Store v14 reuse', () => {
    const contracts = jsonFixture('contracts.v1.json');
    expect(contracts.storeDecision).toMatchObject({
      storeVersion: 14,
      migrationRequired: false,
    });
    expect(contracts.contextViews.selectedJoinMain.steps).not.toHaveProperty(
      'recordAnalytics'
    );
    expect(contracts.contextViews.noneJoinMain.steps).toEqual({
      prepare: { ready: true },
    });
    expect(contracts.contextViews.terminalLifecycle.steps).toHaveProperty(
      'recordAnalytics'
    );
    expect(contracts.outcomes.lateUnjoinedFailure).toMatchObject({
      status: 'failed',
      mainResultRecorded: true,
      publicResult: 'unavailable',
    });
    expect(contracts.lifecycleOrder.joinedFailure).toEqual([
      'owned-branches-settled',
      'on-failure',
      'on-complete',
      'run-finalized',
    ]);
    expect(contracts.recovery).toMatchObject({
      sourceOfTruth: 'folded-event-history',
      ambiguousExternalEffect: 'fail-closed',
      openedForkBlocksRetention: true,
    });
  });

  test('reviews all/selected/none lowering profiles, root-first and terminal forks, recursive rejection, and path-stable choice results', () => {
    const profiles = jsonFixture('compiled-join-profiles.v13.json');
    expect(profiles.all.joinedBranchIds).toEqual([
      'instagram',
      'facebook',
      'analytics',
    ]);
    expect(profiles.selected).toEqual({
      joinedBranchIds: ['instagram', 'facebook'],
      mainVisibleStepIds: [
        'prepare',
        'instagramOutcome',
        'publishFacebook',
      ],
    });
    expect(profiles.none).toEqual({
      joinedBranchIds: [],
      mainVisibleStepIds: ['prepare'],
    });

    for (const name of [
      'join-all.woml',
      'join-selected.woml',
      'join-none.woml',
      'root-first.woml',
      'terminal-fork.woml',
      'nested-fork.invalid.woml',
    ]) {
      expect(parseWoml(fixture(name), { file: name }).root.name).toBe('woml');
    }

    const choiceResults = jsonFixture('choice-results.v1.json');
    expect(choiceResults.instagramEnabledTrue.mergedResult).toEqual({
      status: 'published',
    });
    expect(choiceResults.instagramEnabledFalse.mergedResult).toEqual({
      status: 'skipped',
    });
    expect(
      Object.keys(choiceResults.instagramEnabledTrue.downstreamResult)
    ).toEqual(
      Object.keys(choiceResults.instagramEnabledFalse.downstreamResult)
    );
  });

  test('keeps secrets, copied context, outputs, and error messages out of Event v12', () => {
    const durable = fixture('histories.v12.json');
    for (const forbidden of [
      'context',
      'payload',
      'secret',
      'credentials',
      'messageBody',
      'stack',
      'result',
    ]) {
      expect(durable).not.toContain(forbidden);
    }
  });
});

describe('FJ1 canonical conditional choice migration', () => {
  test('canonical <choose> and legacy conditional <branch> lower identically', () => {
    const canonicalDocument = parseWoml(sharedFixture('branch.woml'), {
      file: 'branch.woml',
    });
    const legacyDocument = parseWoml(sharedFixture('branch.legacy.woml'), {
      file: 'branch.legacy.woml',
    });

    expect(compileWoml(canonicalDocument)).toEqual(
      compileWoml(legacyDocument)
    );
    expect(inspectWomlMigrationDiagnostics(canonicalDocument)).toEqual([]);
    expect(inspectWomlMigrationDiagnostics(legacyDocument)).toMatchObject([
      {
        severity: 'warning',
        code: 'WOML_DEPRECATED_CONDITIONAL_BRANCH',
        phase: 'validation',
        file: 'branch.legacy.woml',
      },
    ]);
  });

  test('accepts result-producing <choose> in every currently legal nested location', () => {
    const source = `<woml><workflow id="nested-choice">
      <triggers><manual id="start" /></triggers>
      <steps>
        <step id="ready"><script>return true;</script></step>
        <approval id="review">
          <when-approved>
            <choose id="approvedRoute">
              <when test="{{context.steps.ready}}"><step id="yes"><script>return 1;</script></step><result value="{{context.steps.yes}}" /></when>
              <otherwise><step id="no"><script>return 0;</script></step><result value="{{context.steps.no}}" /></otherwise>
            </choose>
          </when-approved>
          <when-rejected />
        </approval>
        <step id="finish"><script>return context.steps.review;</script></step>
      </steps>
    </workflow></woml>`;
    const compiled = compileWoml(parseWoml(source, { file: 'nested.woml' }));
    expect(compiled.graph.nodes.map(node => node.id)).toContain(
      'approvedRoute'
    );
  });

  test('uses choice terminology and stable choice diagnostics for canonical source', () => {
    const source = `<woml><workflow id="invalid-choice"><triggers><manual id="start" /></triggers><steps>
      <choose id="decision"><otherwise><step id="no"><script>return 0;</script></step><result value="{{context.steps.no}}" /></otherwise></choose>
    </steps></workflow></woml>`;
    try {
      compileWoml(parseWoml(source, { file: 'invalid-choice.woml' }));
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      const diagnostic = (error as WomlValidationError).diagnostic;
      expect(diagnostic.code).toBe('WOML_CHOOSE_WHEN_REQUIRED');
      expect(diagnostic.message).toContain('<choose');
      return;
    }
    throw new Error('Expected canonical choice validation to fail.');
  });
});
