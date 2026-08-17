import { describe, expect, test } from 'bun:test';

import {
  generateWomlEditorDeclarations,
  withWomlModuleTestRuntime,
} from '../src';

describe('Local module DX', () => {
  test('generates self-contained built-in and imported service autocomplete', () => {
    const declarations = generateWomlEditorDeclarations([
      { name: 'spreadsheet', exports: ['read', 'removeEmptyRows'] },
      { name: 'openai', exports: ['chat'] },
    ]);
    for (const required of [
      'declare const services',
      'readonly http',
      'readonly db',
      'readonly storage',
      'readonly cache',
      'readonly events',
      'readonly workflows',
      'readonly call',
      'readonly "spreadsheet"',
      'readonly "removeEmptyRows"',
      'readonly "openai"',
      'readonly "chat"',
    ]) {
      expect(declarations).toContain(required);
    }
    expect(declarations).not.toContain('declare const context');
    expect(declarations).not.toContain('declare const secrets');
    expect(declarations).not.toContain('declare const attempt');
  });

  test('installs read-only mocked services for ordinary Bun module tests', async () => {
    let calls = 0;
    const result = await withWomlModuleTestRuntime(
      {
        services: {
          openai: {
            chat: async (message: string) => {
              calls += 1;
              return { message: message.toUpperCase() };
            },
          },
        },
      },
      async () => {
        const injected = (
          globalThis as unknown as {
            services: {
              openai: { chat(message: string): Promise<{ message: string }> };
            };
          }
        ).services;
        expect(() => {
          (injected as unknown as Record<string, unknown>).openai = {};
        }).toThrow();
        return await injected.openai.chat('hello');
      }
    );
    expect(result).toEqual({ message: 'HELLO' });
    expect(calls).toBe(1);
    expect(globalThis).not.toHaveProperty('services');
  });

  test('rejects overlapping global test runtimes', async () => {
    await withWomlModuleTestRuntime({ services: {} }, async () => {
      await expect(
        withWomlModuleTestRuntime({ services: {} }, async () => true)
      ).rejects.toThrow('cannot overlap');
    });
  });
});
