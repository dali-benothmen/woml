#!/usr/bin/env bun

const [command] = Bun.argv.slice(2);
if (command !== 'classify') {
  console.error('Usage: ai-review.ts classify');
  process.exit(2);
}
console.log(JSON.stringify({
  model: 'deterministic-rehearsal-v1',
  risk: 'medium',
  confidence: 1,
  reasons: ['production target', 'three non-destructive resource changes'],
}));
