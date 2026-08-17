import { describe, expect, test } from 'bun:test';

import {
  queryRuntimePresentation,
  RuntimeControlError,
} from '../src/runtime-control';

describe('authenticated Run Presentation administration surface', () => {
  test('routes exact run and workflow presentation queries', async () => {
    const calls: string[] = [];
    const presentations = {
      run: (runId: string) => {
        calls.push(`run:${runId}`);
        return { profile: 'woml.run-presentation/v1', runId };
      },
      workflow: (workflowId: string, limit: number) => {
        calls.push(`workflow:${workflowId}:${limit}`);
        return {
          profile: 'woml.run-presentation-list/v1',
          workflowId,
          runs: [],
        };
      },
    };
    expect(await queryRuntimePresentation(
      new URL('http://127.0.0.1/v1/presentations/runs/run_example'),
      presentations
    )).toEqual({
      profile: 'woml.run-presentation/v1',
      runId: 'run_example',
    });
    expect(await queryRuntimePresentation(
      new URL('http://127.0.0.1/v1/presentations/workflows/orders?limit=4'),
      presentations
    )).toEqual({
      profile: 'woml.run-presentation-list/v1',
      workflowId: 'orders',
      runs: [],
    });
    expect(calls).toEqual(['run:run_example', 'workflow:orders:4']);
  });

  test('rejects malformed subjects and invalid workflow history limits', async () => {
    const presentations = { run: () => ({}), workflow: () => ({}) };
    for (const path of [
      '/v1/presentations/runs/not-a-run',
      '/v1/presentations/workflows/orders?limit=11',
      '/v1/presentations/workflows/orders?limit=nope',
    ]) {
      try {
        await queryRuntimePresentation(new URL(`http://127.0.0.1${path}`), presentations);
        throw new Error('Expected the presentation query to fail.');
      } catch (error) {
        expect(error).toBeInstanceOf(RuntimeControlError);
        expect((error as RuntimeControlError).code).toMatch(/^WOML_/);
      }
    }
  });
});
