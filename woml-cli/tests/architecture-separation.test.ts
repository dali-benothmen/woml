import { describe, expect, test } from 'bun:test';

import {
  cronflowRuntimeDependencies,
  importSpecifiers,
  isLegacySdkSpecifier,
} from '../scripts/verify-architecture-separation';

describe('WOML architecture separation scanner', () => {
  test('detects static, dynamic, and CommonJS SDK imports', () => {
    const source = `
      import { cronflow } from '../../sdk';
      export { define } from '@cronflow/sdk';
      const legacy = await import('../sdk/src/cronflow');
      const root = require('cronflow');
    `;
    expect(importSpecifiers(source)).toEqual([
      '../../sdk',
      '@cronflow/sdk',
      '../sdk/src/cronflow',
      'cronflow',
    ]);
    expect(importSpecifiers(source).every(isLegacySdkSpecifier)).toBe(true);
    expect(isLegacySdkSpecifier('../woml/src/compiler')).toBe(false);
  });

  test('rejects runtime @cronflow dependencies without confusing metadata', () => {
    expect(
      cronflowRuntimeDependencies({
        dependencies: { woml: '1.0.0' },
        optionalDependencies: { '@cronflow/linux-x64': '1.0.0' },
        peerDependencies: { helper: 'file:../sdk/helper' },
        devDependencies: { '@cronflow/test-helper': '1.0.0' },
        repository: 'https://github.com/example/cronflow',
      })
    ).toEqual(['@cronflow/linux-x64', 'helper']);
  });
});
