import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlExecutableDefinitionPackage,
  canonicalizeWomlDefinitionPackage,
  compileWoml,
  inspectCompiledWorkflowGraph,
  inspectWomlMigrationDiagnostics,
  parseWoml,
  validateWoml,
  WomlCompileError,
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

describe('Frozen fork and branch contracts', () => {
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
      'on-error',
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

describe('Canonical conditional choice migration', () => {
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

function fj2Workflow(body: string): string {
  return `<woml><workflow id="fj2-test"><triggers><manual id="start" /></triggers><steps>${body}</steps></workflow></woml>`;
}

function fj2Diagnostic(source: string): string {
  const document = parseWoml(source, { file: 'fj2-invalid.woml' });
  try {
    validateWoml(document);
  } catch (error) {
    if (error instanceof WomlValidationError || error instanceof WomlCompileError) {
      return error.diagnostic.code;
    }
    throw error;
  }
  throw new Error('Expected  validation to fail.');
}

describe('Fork and branch authoring', () => {
  test('accepts all reviewed forks, including omitted/all/selected/none joins, root-first, terminal, and multi-step branches', () => {
    for (const name of [
      'join-all.woml',
      'join-selected.woml',
      'join-none.woml',
      'root-first.woml',
      'terminal-fork.woml',
    ]) {
      const document = parseWoml(fixture(name), { file: name });
      expect(() => validateWoml(document), name).not.toThrow();
      expect(compileWoml(document).schemaVersion).toBe(13);
    }
  });

  test('accepts branch IDs reused in different forks and canonicalizes selected joins independently of attribute order', () => {
    const source = fj2Workflow(`
      <step id="before"><script>return true;</script></step>
      <fork id="first" join="beta alpha">
        <branch id="alpha"><step id="alphaOne"><script>return 1;</script></step></branch>
        <branch id="beta"><step id="betaOne"><script>return 2;</script></step></branch>
      </fork>
      <fork id="second" join="all">
        <branch id="alpha"><step id="alphaTwo"><script>return 3;</script></step></branch>
      </fork>
      <step id="finish"><script>return context.steps.alphaTwo;</script></step>`);
    expect(() =>
      validateWoml(parseWoml(source, { file: 'reused-branch-id.woml' }))
    ).not.toThrow();
  });

  test('validates exact control-only choice shape for Model v13 lowering', () => {
    const source = fj2Workflow(`
      <step id="ready"><script>return true;</script></step>
      <choose>
        <when test="{{context.steps.ready}}"><step id="yes"><script>return 1;</script></step></when>
        <otherwise><step id="no"><script>return 0;</script></step></otherwise>
      </choose>
      <step id="finish"><script>return context.steps.ready;</script></step>`).replace(
      'id="fj2-test"',
      'id="control-choice"'
    );
    const document = parseWoml(source, { file: 'control-choice.woml' });
    expect(() => validateWoml(document)).not.toThrow();
    const compiled = compileWoml(document);
    expect(compiled.schemaVersion).toBe(13);
    if (compiled.schemaVersion !== 13) throw new Error('Expected Model v13.');
    expect(compiled.graph.choices[0].choiceId).toBe(
      '__woml_choice__root_1'
    );
  });

  test('rejects malformed fork and branch structure at the responsible source', () => {
    const cases = [
      [
        fj2Workflow('<fork><branch id="a"><step id="x"><script>return 1;</script></step></branch></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_MISSING_ATTRIBUTE',
      ],
      [
        fj2Workflow('<fork id="empty"></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_FORK_EMPTY',
      ],
      [
        fj2Workflow('<fork id="bad"><step id="x"><script>return 1;</script></step></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_FORK_CHILD_INVALID',
      ],
      [
        fj2Workflow('<fork id="bad"><branch id="empty"></branch></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_FORK_BRANCH_EMPTY',
      ],
      [
        fj2Workflow('<fork id="bad"><branch id="all"><step id="x"><script>return 1;</script></step></branch></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_FORK_BRANCH_ID_RESERVED',
      ],
      [
        fj2Workflow('<fork id="bad"><branch id="a"><step id="x"><script>return 1;</script></step></branch><branch id="a"><step id="y"><script>return 2;</script></step></branch></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_FORK_BRANCH_ID_DUPLICATE',
      ],
      [
        fj2Workflow('<fork id="bad"><branch id="a" name="No"><step id="x"><script>return 1;</script></step></branch></fork><step id="end"><script>return 1;</script></step>'),
        'WOML_FORK_BRANCH_ATTRIBUTE_UNSUPPORTED',
      ],
      [
        fj2Workflow('<branch id="route"><step id="x"><script>return 1;</script></step></branch>'),
        'WOML_FORK_BRANCH_PLACEMENT_INVALID',
      ],
    ] as const;
    for (const [source, code] of cases) {
      expect(fj2Diagnostic(source), code).toBe(code);
    }
  });

  test('rejects empty, mixed, duplicate, malformed, and unknown join values', () => {
    const fork = (join: string) =>
      fj2Workflow(`<fork id="distribution" join="${join}">
        <branch id="instagram"><step id="ig"><script>return 1;</script></step></branch>
        <branch id="facebook"><step id="fb"><script>return 2;</script></step></branch>
      </fork><step id="finish"><script>return true;</script></step>`);
    for (const [join, code] of [
      ['', 'WOML_FORK_JOIN_INVALID'],
      ['all instagram', 'WOML_FORK_JOIN_INVALID'],
      ['none facebook', 'WOML_FORK_JOIN_INVALID'],
      ['instagram instagram', 'WOML_FORK_JOIN_DUPLICATE'],
      ['instagram tiktok', 'WOML_FORK_JOIN_UNKNOWN_BRANCH'],
      ['instagram,facebook', 'WOML_FORK_JOIN_INVALID'],
    ] as const) {
      expect(fj2Diagnostic(fork(join)), join).toBe(code);
    }
  });

  test('rejects a nested fork throughout a fork branch subtree', () => {
    expect(fj2Diagnostic(fixture('nested-fork.invalid.woml'))).toBe(
      'WOML_FORK_NESTED_UNSUPPORTED'
    );
    const approvalNested = fj2Workflow(`
      <step id="before"><script>return true;</script></step>
      <fork id="outer"><branch id="route">
        <approval id="review"><when-approved>
          <fork id="nested"><branch id="inside"><step id="x"><script>return 1;</script></step></branch></fork>
        </when-approved><when-rejected /></approval>
      </branch></fork>
      <step id="finish"><script>return true;</script></step>`);
    expect(fj2Diagnostic(approvalNested)).toBe(
      'WOML_FORK_NESTED_UNSUPPORTED'
    );
  });

  test('rejects invalid control-only choice results, metadata, and empty arms', () => {
    const cases = [
      [
        fj2Workflow('<step id="ready"><script>return true;</script></step><choose><when test="{{context.steps.ready}}"><step id="x"><script>return 1;</script></step><result value="{{context.steps.x}}" /></when><otherwise><step id="y"><script>return 0;</script></step></otherwise></choose><step id="end"><script>return true;</script></step>'),
        'WOML_CHOOSE_RESULT_REQUIRES_ID',
      ],
      [
        fj2Workflow('<step id="ready"><script>return true;</script></step><choose name="Route"><when test="{{context.steps.ready}}"><step id="x"><script>return 1;</script></step></when><otherwise><step id="y"><script>return 0;</script></step></otherwise></choose><step id="end"><script>return true;</script></step>'),
        'WOML_CHOOSE_METADATA_REQUIRES_ID',
      ],
      [
        fj2Workflow('<step id="ready"><script>return true;</script></step><choose><when test="{{context.steps.ready}}"></when><otherwise><step id="y"><script>return 0;</script></step></otherwise></choose><step id="end"><script>return true;</script></step>'),
        'WOML_CHOOSE_ARM_EMPTY',
      ],
    ] as const;
    for (const [source, code] of cases) {
      expect(fj2Diagnostic(source)).toBe(code);
    }
  });

  test('enforces sibling and unjoined visibility while allowing joined outputs downstream', () => {
    const siblingReference = fj2Workflow(`
      <step id="ready"><script>return true;</script></step>
      <fork id="distribution" join="all">
        <branch id="instagram"><choose><when test="{{context.steps.facebookPost}}"><step id="ig"><script>return 1;</script></step></when><otherwise><step id="igSkip"><script>return 0;</script></step></otherwise></choose></branch>
        <branch id="facebook"><step id="facebookPost"><script>return true;</script></step></branch>
      </fork>
      <step id="finish"><script>return true;</script></step>`);
    expect(fj2Diagnostic(siblingReference)).toBe(
      'WOML_FORK_REFERENCE_NOT_VISIBLE'
    );

    const unjoinedReference = fj2Workflow(`
      <step id="ready"><script>return true;</script></step>
      <fork id="distribution" join="instagram">
        <branch id="instagram"><step id="instagramPost"><script>return true;</script></step></branch>
        <branch id="analytics"><step id="analyticsPost"><script>return true;</script></step></branch>
      </fork>
      <choose><when test="{{context.steps.analyticsPost}}"><step id="yes"><script>return 1;</script></step></when><otherwise><step id="no"><script>return 0;</script></step></otherwise></choose>
      <step id="finish"><script>return true;</script></step>`);
    expect(fj2Diagnostic(unjoinedReference)).toBe(
      'WOML_FORK_REFERENCE_NOT_VISIBLE'
    );

    const joinedReference = unjoinedReference
      .replace('join="instagram"', 'join="all"')
      .replace('context.steps.analyticsPost', 'context.steps.instagramPost');
    expect(() =>
      validateWoml(parseWoml(joinedReference, { file: 'joined.woml' }))
    ).not.toThrow();
  });

  test('rejects a terminal fork without an earlier main result and keeps a reviewed terminal fork valid', () => {
    const invalid = fj2Workflow(
      '<fork id="only"><branch id="work"><step id="x"><script>return 1;</script></step></branch></fork>'
    );
    expect(fj2Diagnostic(invalid)).toBe(
      'WOML_FORK_TERMINAL_RESULT_REQUIRED'
    );
    expect(() =>
      validateWoml(
        parseWoml(fixture('terminal-fork.woml'), {
          file: 'terminal-fork.woml',
        })
      )
    ).not.toThrow();
  });
});

describe('Model v13 lowering', () => {
  test('deep-equals the corrected reviewed join-all graph', () => {
    const source = fixture('join-all.woml');
    const compiled = compileWoml(
      parseWoml(source, { file: 'join-all.woml' })
    );
    expect(compiled).toEqual(jsonFixture('join-all.compiled.v13.json'));
    expect(inspectCompiledWorkflowGraph(compiled.graph, {
      requireSingleTerminal: true,
    })).toEqual([]);
  });

  test('lowers selected and none joins with deterministic visibility', () => {
    const selected = compileWoml(
      parseWoml(fixture('join-selected.woml'), {
        file: 'join-selected.woml',
      })
    );
    const none = compileWoml(
      parseWoml(fixture('join-none.woml'), { file: 'join-none.woml' })
    );
    if (selected.schemaVersion !== 13 || none.schemaVersion !== 13) {
      throw new Error('Expected Model v13.');
    }
    expect(selected.graph.forks[0].joinedBranchIds).toEqual([
      'instagram',
      'facebook',
    ]);
    expect(
      selected.graph.contextVisibility.find(item => item.nodeId === 'finish')
        ?.stepIds
    ).toEqual(['prepare', 'instagramOutcome', 'publishFacebook']);
    expect(none.graph.forks[0].joinedBranchIds).toEqual([]);
    expect(
      none.graph.edges.some(
        edge =>
          edge.from === '__woml_fork__analyticsFork__open' &&
          edge.to === '__woml_fork__analyticsFork__join'
      )
    ).toBe(true);
    expect(
      none.graph.contextVisibility.find(item => item.nodeId === 'finish')
        ?.stepIds
    ).toEqual(['prepare']);
  });

  test('lowers control-only choice identities without publishing an output', () => {
    const source = fj2Workflow(`
      <step id="ready"><script>return true;</script></step>
      <choose>
        <when test="{{context.steps.ready}}"><step id="yes"><script>return 1;</script></step></when>
        <otherwise><step id="no"><script>return 0;</script></step></otherwise>
      </choose>
      <step id="finish"><script>return context.steps.ready;</script></step>`).replace(
      'id="fj2-test"',
      'id="control-choice"'
    );
    const compiled = compileWoml(
      parseWoml(source, { file: 'control-choice-v13.woml' })
    );
    if (compiled.schemaVersion !== 13) throw new Error('Expected Model v13.');
    expect(compiled.graph.choices).toEqual([
      {
        choiceId: '__woml_choice__root_1',
        selectorNodeId: '__woml_choice__root_1__select',
        joinNodeId: '__woml_choice__root_1__join',
        armIds: [
          '__woml_choice__root_1:when:0',
          '__woml_choice__root_1:otherwise',
        ],
      },
    ]);
    expect(
      compiled.graph.contextVisibility.find(item => item.nodeId === 'finish')
        ?.stepIds
    ).toEqual(['ready']);
    expect(compiled.graph.settlement.ownedBranchTerminalNodeIds).toEqual([]);
    expect(inspectCompiledWorkflowGraph(compiled.graph, {
      requireSingleTerminal: true,
    })).toEqual([]);
    const validateModel = contractValidators().getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v13'
    )!;
    expect(
      validateModel(compiled),
      JSON.stringify(validateModel.errors, null, 2)
    ).toBe(true);
    expect(compiled).toEqual(
      jsonFixture('control-choice.compiled.v13.json')
    );
  });

  test('preserves root-first entry and a prior result for a terminal fork', () => {
    const rootFirst = compileWoml(
      parseWoml(fixture('root-first.woml'), { file: 'root-first.woml' })
    );
    const terminal = compileWoml(
      parseWoml(fixture('terminal-fork.woml'), {
        file: 'terminal-fork.woml',
      })
    );
    if (rootFirst.schemaVersion !== 13 || terminal.schemaVersion !== 13) {
      throw new Error('Expected Model v13.');
    }
    expect(rootFirst.graph.entryNodeIds).toEqual([
      '__woml_fork__initialFanout__open',
    ]);
    expect(terminal.graph.settlement.mainResultNodeId).toBe('mainResult');
    expect(
      terminal.graph.edges.find(
        edge => edge.id === '__woml_workflow__:main:settlement'
      )?.from
    ).toBe('__woml_fork__terminalDistribution__join');
  });

  test('rejects malformed ownership, visibility, choice, and settlement graphs', () => {
    const compiled = compileWoml(
      parseWoml(fixture('join-all.woml'), { file: 'join-all.woml' })
    );
    if (compiled.schemaVersion !== 13) throw new Error('Expected Model v13.');
    const cases = [
      ['INVALID_FORK_GRAPH', (graph: any) => graph.forks[0].joinedBranchIds.push('missing')],
      ['INVALID_CONTEXT_VISIBILITY', (graph: any) => graph.contextVisibility.pop()],
      ['INVALID_WORKFLOW_SETTLEMENT', (graph: any) => graph.settlement.mainResultNodeId = 'missing'],
      ['INVALID_FORK_GRAPH', (graph: any) => graph.forks[0].branches[1].entryNodeId = 'publishInstagram'],
    ] as const;
    for (const [code, mutate] of cases) {
      const graph = structuredClone(compiled.graph);
      mutate(graph);
      expect(
        inspectCompiledWorkflowGraph(graph, { requireSingleTerminal: true }).some(
          issue => issue.code === code
        ),
        code
      ).toBe(true);
    }
  });

  test('emits deterministic Definition Package v8 for module-backed forks', async () => {
    const sourcePath = resolve(fixtureRoot, 'module-fork.woml');
    const document = parseWoml(readFileSync(sourcePath, 'utf8'), {
      file: sourcePath,
    });
    const options = { sourcePath, projectRoot: sharedFixtureRoot };
    const first = await buildWomlExecutableDefinitionPackage(document, options);
    const second = await buildWomlExecutableDefinitionPackage(document, options);
    expect(first.schemaVersion).toBe(8);
    expect(first.profile).toBe('woml.definition-package/v8');
    expect(first.workflow.model.schemaVersion).toBe(13);
    expect(first.runtimeReady).toBe(false);
    expect(first.rootHash).toBe(second.rootHash);
    expect(canonicalizeWomlDefinitionPackage(first)).toBe(
      canonicalizeWomlDefinitionPackage(second)
    );
  });
});
