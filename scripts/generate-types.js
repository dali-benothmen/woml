#!/usr/bin/env node

/**
 * Generate TypeScript declaration files for the built packages
 * This is needed because Bun build doesn't generate .d.ts files
 */

const fs = require('fs');
const path = require('path');

console.log('📝 Generating TypeScript declaration files...');

// Create basic declaration files
const declarations = {
  'dist/index.d.ts': `/**
 * Node-Cronflow Main Package
 */

export * from './sdk/index';

export { cronflow as default } from './sdk/index';
`,

  'dist/sdk/index.d.ts': `/**
 * Node-Cronflow SDK
 */

export const VERSION: string;

export interface WorkflowDefinition {
  id: string;
  name: string;
  description?: string;
  steps: StepDefinition[];
  triggers: TriggerDefinition[];
  created_at: Date;
  updated_at: Date;
}

export interface StepDefinition {
  id: string;
  name: string;
  action: string;
  timeout?: number;
  retry?: RetryConfig;
  depends_on: string[];
}

export interface RetryConfig {
  max_attempts: number;
  backoff_ms: number;
}

export type TriggerDefinition =
  | { type: 'webhook'; path: string; method: string }
  | { type: 'schedule'; cron_expression: string }
  | { type: 'manual' };

export const cronflow: Cronflow;
export type { Cronflow, WorkflowInstance };
`,
};

// Write declaration files
for (const [filePath, content] of Object.entries(declarations)) {
  const fullPath = path.join(__dirname, '..', filePath);
  const dir = path.dirname(fullPath);

  // Ensure directory exists
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }

  fs.writeFileSync(fullPath, content);
  console.log(`✅ Generated: ${filePath}`);
}

console.log('🎉 All TypeScript declaration files generated!');
