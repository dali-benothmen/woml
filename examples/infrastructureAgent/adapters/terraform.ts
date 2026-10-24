#!/usr/bin/env bun

import { mkdir } from 'node:fs/promises';

const [command, ...arguments_] = Bun.argv.slice(2);
const output = arguments_.find(value => value.startsWith('--output='))?.slice(9);

if (command === 'plan' && output !== undefined) {
  await mkdir(new URL('../.woml/artifact-input/', import.meta.url), {
    recursive: true,
  });
  const plan = {
    profile: 'woml.fake-terraform-plan/v1',
    environment: 'production-rehearsal',
    create: 1,
    update: 2,
    delete: 0,
    resources: ['module.payments.service', 'module.payments.alert'],
  };
  await Bun.write(new URL(`../${output}`, import.meta.url), `${JSON.stringify(plan, null, 2)}\n`);
  console.log(JSON.stringify({ command, output, changes: 3, destructive: false }));
  process.exit(0);
}

if (command === 'apply') {
  console.log('Applying reviewed rehearsal plan');
  console.log('Progress: 1/3 prepare');
  console.log('Progress: 2/3 apply');
  console.log('Progress: 3/3 complete');
  console.log(JSON.stringify({ command, releaseId: 'release-rehearsal-42', changed: false }));
  process.exit(0);
}

if (command === 'rollback') {
  console.log(JSON.stringify({ command, releaseId: 'release-rehearsal-42', restored: true }));
  process.exit(0);
}

console.error(`Unsupported fake Terraform command: ${command ?? '<missing>'}`);
process.exit(2);
