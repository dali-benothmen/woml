// Versioned, deterministic investigation inputs. Generated .woml sources and
// their hashes are retained with each experiment, not reconstructed from logs.
export interface RuntimeProfileFixture {
  readonly id: string;
  readonly source: string;
  readonly expected: { readonly value: number };
  readonly governed: boolean;
}

function document(id: string, body: string, lifecycle = ''): string {
  return `<woml>
  <workflow id="runtime-profile-${id}" version="1.0.0">
    <config concurrency="1" timeout="3m" />
    ${lifecycle}
    <triggers><manual id="start" /></triggers>
    <steps>${body}</steps>
  </workflow>
</woml>\n`;
}

function step(id: string, code: string): string {
  return `\n      <step id="${id}"><script>${code}</script></step>`;
}

function sequential(count: number, hooks = false): RuntimeProfileFixture {
  const id = hooks ? `hooks-${count}` : `sequential-${count}`;
  const body = Array.from({ length: count }, (_, index) => step(`step${index + 1}`, `return { value: ${index + 1} };`)).join('');
  const lifecycle = hooks ? `<lifecycle>
      <on-start><script>return null;</script></on-start>
      <on-step-start steps="step4"><script>return null;</script></on-step-start>
      <on-step-complete steps="step4"><script>return null;</script></on-step-complete>
      <on-complete><script>return null;</script></on-complete>
    </lifecycle>` : '';
  return { id, source: document(id, body, lifecycle), expected: { value: count }, governed: false };
}

export function runtimeProfileFixtures(): readonly RuntimeProfileFixture[] {
  const state = Array.from({ length: 8 }, (_, index) => step(`step${index + 1}`, `const saved = await services.state.increment("counter", 1, { name: "count" }); if (saved.value !== ${index + 1}) throw new Error("State isolation or ordering failed"); return { value: saved.value };`)).join('');
  const storage = step('create', 'return await services.storage.put({ key: "profile/item.json", value: { revision: 1 } }, { name: "create" });')
    + step('replace', 'return await services.storage.put({ key: "profile/item.json", value: { revision: 2 }, ifVersion: context.steps.create.version }, { name: "replace" });')
    + step('read', 'const saved = await services.storage.get({ key: "profile/item.json", responseType: "json", ifVersion: context.steps.replace.version }); if (saved.data.revision !== 2) throw new Error("Storage revision mismatch"); return { verified: true };')
    + step('remove', 'const removed = await services.storage.delete({ key: "profile/item.json", ifVersion: context.steps.replace.version }, { name: "remove" }); if (!removed.deleted) throw new Error("Storage deletion failed"); return { value: 4 };');
  const parallel = `<parallel id="fanout" concurrency="4" on-error="wait-all">${Array.from({ length: 8 }, (_, index) => step(`branch${index + 1}`, 'return { value: 1 };')).join('')}</parallel>`
    + step('summary', `const values = [${Array.from({ length: 8 }, (_, index) => `context.steps.branch${index + 1}.value`).join(',')}]; return { value: values.reduce((sum, value) => sum + value, 0) };`);
  const loop = step('items', 'return { items: [0,1,2,3,4,5,6,7] };') + `
      <for-each id="itemsProcessed" items="{{context.steps.items.items}}" concurrency="4">
        <step id="item"><script>return { index: context.iteration.index, value: context.item };</script></step>
        <result value="{{context.steps.item}}" />
      </for-each>` + step('summary', 'const loop = context.steps.itemsProcessed; if (loop.succeeded !== 8 || loop.results.some((item, index) => item.index !== index || item.value !== index)) throw new Error("Loop ordering failed"); return { value: loop.succeeded };');
  const combined = step('prepare', 'await services.state.set("preview", { status: "prepared" }, { name: "prepare" }); return { preferred: true, strategy: "canary", zones: ["north", "south", "west"] };') + `
      <parallel id="checks" concurrency="2">
        <step id="stateCheck"><script>const saved = await services.state.get("preview"); if (!saved.found) throw new Error("Missing preview"); return { ready: true };</script></step>
        <step id="pureCheck"><script>return { ready: true };</script></step>
      </parallel>
      <choose id="route">
        <when test="{{context.steps.prepare.preferred}}">
          <step id="preferred"><script>return { route: "preferred" };</script></step>
          <result value="{{context.steps.preferred}}" />
        </when>
        <otherwise>
          <step id="fallback"><script>return { route: "fallback" };</script></step>
          <result value="{{context.steps.fallback}}" />
        </otherwise>
      </choose>
      <switch id="strategy" value="{{context.steps.prepare.strategy}}">
        <case value="canary">
          <step id="canary"><script>return { strategy: "canary" };</script></step>
          <result value="{{context.steps.canary}}" />
        </case>
        <default>
          <step id="standard"><script>return { strategy: "standard" };</script></step>
          <result value="{{context.steps.standard}}" />
        </default>
      </switch>
      <for-each id="zones" items="{{context.steps.prepare.zones}}" concurrency="2">
        <step id="zone"><script>return { zone: context.item, index: context.iteration.index };</script></step>
        <result value="{{context.steps.zone}}" />
      </for-each>
      <lock id="previewLock" key="profiling/local-preview" acquire-timeout="10s" lease="2m">
        <saga id="release">
          <step id="deploy">
            <script>await services.state.set("preview", { status: "deployed", fence: context.lock.fencingToken }, { name: "deploy" }); return { deployed: true };</script>
            <compensate timeout="30s"><script>await services.state.set("preview", { status: "compensated" }, { name: "compensate" }); return { corrected: true };</script></compensate>
          </step>
          <step id="verify"><script>const saved = await services.state.get("preview"); if (!saved.found || saved.value.status !== "deployed") throw new Error("Preview not deployed"); return { verified: true };</script></step>
          <cleanup>
            <step id="cleanup" timeout="30s"><script>await services.state.increment("cleanup", 1, { name: "cleanup" }); await services.cache.set("cleanup", { done: true }, { name: "cache-cleanup" }); return { cleaned: true };</script></step>
          </cleanup>
        </saga>
      </lock>` + step('summary', 'const cleanup = await services.state.get("cleanup"); if (!cleanup.found || cleanup.value !== 1 || context.steps.zones.succeeded !== 3 || context.steps.route.route !== "preferred" || context.steps.strategy.strategy !== "canary") throw new Error("Combined invariants failed"); return { value: context.steps.zones.succeeded };');
  return [
    sequential(2), sequential(8), sequential(16), sequential(32), sequential(8, true),
    { id: 'state-8', source: document('state-8', state), expected: { value: 8 }, governed: true },
    { id: 'storage-4', source: document('storage-4', storage), expected: { value: 4 }, governed: true },
    { id: 'parallel-8', source: document('parallel-8', parallel), expected: { value: 8 }, governed: false },
    { id: 'foreach-8', source: document('foreach-8', loop), expected: { value: 8 }, governed: false },
    { id: 'combined', source: document('combined', combined, '<lifecycle><on-start><script>return null;</script></on-start><on-complete><script>return null;</script></on-complete></lifecycle>'), expected: { value: 3 }, governed: true },
  ];
}

export const runtimeProfilePolicy = {
  contract: 'woml.infrastructure-policy', contractVersion: 1,
  id: 'runtime-profiling-local', version: '1', environment: 'local-profiling',
  defaults: { read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' },
  rules: [{ id: 'local-state-storage-cache-locks', match: {
    workflows: ['runtime-profile-state-8', 'runtime-profile-storage-4', 'runtime-profile-combined'],
    capabilities: ['state', 'storage', 'cache', 'locks'],
  }, decision: 'allow' }],
} as const;
