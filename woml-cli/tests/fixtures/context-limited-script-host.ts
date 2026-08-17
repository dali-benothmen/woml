process.env.WOML_SCRIPT_HOST_MAX_CONTEXT_BYTES = '4096';

const { runScriptHost } = await import('../../src/script-host');

await runScriptHost();

export {};
