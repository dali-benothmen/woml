import { readFile } from 'node:fs/promises';

const root = new URL('../../', import.meta.url);
const required = new Map([
  ['woml-cli/src/runtime-inspector.ts', ['WOML INSPECT', '--no-color', 'cancel_run', 'woml list --json']],
  ['woml-cli/src/cli.ts', ["args[0] === 'inspect'", 'inspectUsage']],
  ['docs/woml-observability.md', ['woml inspect']],
]);

for (const [path, markers] of required) {
  const text = await readFile(new URL(path, root), 'utf8');
  for (const marker of markers) {
    if (!text.includes(marker)) throw new Error(`${path} is missing PRO6 marker: ${marker}`);
  }
}

console.log('PRO6 terminal inspector verification passed.');
