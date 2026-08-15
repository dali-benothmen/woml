#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

interface RetirementContract {
  readonly profile: string;
  readonly contractVersion: number;
  readonly publishedAt: string;
  readonly deprecatedPackage: {
    readonly name: string;
    readonly exports: readonly string[];
    readonly finalFeatureVersion: string;
    readonly maintenanceLine: string;
    readonly publicSurfaceFiles: Readonly<Record<string, string>>;
  };
  readonly supportWindow: {
    readonly startsOn: string;
    readonly endsOn: string;
    readonly durationMonths: number;
  };
  readonly maintenancePolicy: {
    readonly newFeatures: boolean;
    readonly allowedChanges: readonly string[];
  };
  readonly breakingBoundary: {
    readonly earliestRemovalVersion: string;
    readonly removalNotBefore: string;
    readonly automaticWorkflowConversion: boolean;
    readonly automaticDataDeletion: boolean;
  };
  readonly replacement: {
    readonly authoringSurface: string;
    readonly command: string;
    readonly stateAuthority: string;
  };
  readonly legacyState: {
    readonly knownDefaultPaths: readonly string[];
    readonly automaticRunHistoryConversion: boolean;
    readonly archiveRequiredBeforeRemoval: boolean;
  };
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function sha256(path: string): Promise<string> {
  const hash = new Bun.CryptoHasher('sha256');
  hash.update(await Bun.file(path).arrayBuffer());
  return hash.digest('hex');
}

export async function verifySdkRetirementContract(root: string): Promise<void> {
  const contractPath = resolve(
    root,
    'docs/contracts/cronflow-sdk-retirement.v1.json'
  );
  const contract = JSON.parse(
    await readFile(contractPath, 'utf8')
  ) as RetirementContract;

  assert(
    contract.profile === 'cronflow.sdk-retirement/v1' &&
      contract.contractVersion === 1,
    'The SDK retirement contract identity changed without a new version.'
  );
  assert(
    contract.publishedAt === '2026-08-15' &&
      contract.supportWindow.startsOn === '2026-08-15' &&
      contract.supportWindow.endsOn === '2027-02-15' &&
      contract.supportWindow.durationMonths === 6,
    'The v1 publication or support window drifted.'
  );
  assert(
    contract.deprecatedPackage.name === 'cronflow' &&
      contract.deprecatedPackage.finalFeatureVersion === '0.11.6' &&
      contract.deprecatedPackage.maintenanceLine === '0.11.x',
    'The frozen Cronflow package/version boundary drifted.'
  );
  assert(
    JSON.stringify(contract.deprecatedPackage.exports) ===
      JSON.stringify(['.', './sdk']),
    'The deprecated npm export surface changed.'
  );
  assert(
    contract.maintenancePolicy.newFeatures === false &&
      contract.maintenancePolicy.allowedChanges.length === 4,
    'The maintenance-only policy changed.'
  );
  assert(
    contract.breakingBoundary.earliestRemovalVersion === '1.0.0' &&
      contract.breakingBoundary.removalNotBefore === '2027-02-16' &&
      contract.breakingBoundary.automaticWorkflowConversion === false &&
      contract.breakingBoundary.automaticDataDeletion === false,
    'The breaking boundary or data-safety promise changed.'
  );
  assert(
    contract.legacyState.archiveRequiredBeforeRemoval &&
      !contract.legacyState.automaticRunHistoryConversion &&
      contract.legacyState.knownDefaultPaths.includes('.cronflow/data.db'),
    'The legacy data archive promise changed.'
  );

  const manifest = JSON.parse(
    await readFile(resolve(root, 'package.json'), 'utf8')
  ) as {
    readonly name: string;
    readonly version: string;
    readonly exports: Readonly<Record<string, unknown>>;
    readonly files: readonly string[];
  };
  assert(
    manifest.name === contract.deprecatedPackage.name &&
      manifest.version === contract.deprecatedPackage.finalFeatureVersion,
    'package.json no longer matches the frozen SDK product identity.'
  );
  assert(
    JSON.stringify(Object.keys(manifest.exports)) ===
      JSON.stringify(contract.deprecatedPackage.exports),
    'package.json changed the frozen deprecated export paths.'
  );

  const shippedDocuments = [
    'docs/cronflow-sdk-retirement.md',
    'docs/cronflow-sdk-data-archive.md',
    'docs/woml-sdk-migration.md',
    'docs/contracts/cronflow-sdk-retirement.v1.json',
  ];
  for (const path of shippedDocuments) {
    assert(manifest.files.includes(path), `${path} is missing from npm files.`);
  }

  for (const [path, expectedHash] of Object.entries(
    contract.deprecatedPackage.publicSurfaceFiles
  )) {
    assert(
      (await sha256(resolve(root, path))) === expectedHash,
      `${path} changed the frozen SDK surface. Publish a reviewed contract version before changing exports.`
    );
  }

  const [sdkSource, readme, retirement, migration, archive] = await Promise.all(
    [
      'sdk/src/cronflow.ts',
      'README.md',
      'docs/cronflow-sdk-retirement.md',
      'docs/woml-sdk-migration.md',
      'docs/cronflow-sdk-data-archive.md',
    ].map(path => readFile(resolve(root, path), 'utf8'))
  );
  assert(
    sdkSource.includes("export const VERSION = '0.11.6';"),
    'The SDK VERSION constant does not match cronflow@0.11.6.'
  );
  for (const [name, source] of [
    ['README', readme],
    ['retirement contract', retirement],
    ['migration guide', migration],
  ] as const) {
    assert(
      source.includes('2027-02-15') || source.includes('February 15, 2027'),
      `${name} does not publish the support end date.`
    );
  }
  assert(
    retirement.includes('Feature-equivalence table') &&
      retirement.includes('Breaking-release boundary') &&
      retirement.includes('No equivalent yet'),
    'The retirement document is missing a required product boundary.'
  );
  assert(
    migration.includes('cronflow-sdk-data-archive.md') &&
      archive.includes('PRAGMA integrity_check') &&
      archive.includes('Never rename the legacy database'),
    'The migration or archive safety procedure is incomplete.'
  );
}

if (import.meta.main) {
  const root = resolve(import.meta.dir, '../..');
  await verifySdkRetirementContract(root);
  console.log(
    'SDK retirement contract verified: cronflow@0.11.6 is maintenance-only through 2027-02-15.'
  );
}
