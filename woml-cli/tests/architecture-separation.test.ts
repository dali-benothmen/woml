import { describe, expect, test } from 'bun:test';

import {
  importSpecifiers,
  isRetiredSdkSpecifier,
  retiredRootManifestViolations,
  retiredSdkRuntimeDependencies,
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
    expect(importSpecifiers(source).every(isRetiredSdkSpecifier)).toBe(true);
    expect(isRetiredSdkSpecifier('../woml/src/compiler')).toBe(false);
  });

  test('rejects runtime @cronflow dependencies without confusing metadata', () => {
    expect(
      retiredSdkRuntimeDependencies({
        dependencies: { woml: '1.0.0' },
        optionalDependencies: { '@cronflow/linux-x64': '1.0.0' },
        peerDependencies: { helper: 'file:../sdk/helper' },
        devDependencies: { '@cronflow/test-helper': '1.0.0' },
        repository: 'https://github.com/example/cronflow',
      })
    ).toEqual(['@cronflow/linux-x64', 'helper']);
  });

  test('keeps the repository root private and without a JavaScript SDK export', () => {
    expect(
      retiredRootManifestViolations({
        name: 'woml-repository',
        private: true,
        scripts: { build: 'bun run --cwd woml-cli build' },
      })
    ).toEqual([]);
    expect(
      retiredRootManifestViolations({
        name: 'cronflow',
        private: false,
        main: 'dist/index.js',
        types: 'dist/index.d.ts',
        exports: { '.': './dist/index.js' },
        optionalDependencies: { '@cronflow/linux-x64': '0.10.4' },
        publishConfig: { access: 'public' },
      })
    ).toEqual([
      'root package is publishable',
      'root package is named cronflow',
      'root package declares main',
      'root package declares types',
      'root package declares exports',
      'root package declares publishConfig',
      'root package declares a retired SDK runtime dependency',
    ]);
  });
});
