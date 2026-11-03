// Diagnostic-only entry. One configuration, one execution, then parent termination.
// No production profiler and no reuse of an executed worker.
import type { ScriptWorkerRequest } from '../src/script-host/worker';

const realmId = crypto.randomUUID();
const post = self.postMessage.bind(self);
let stage = 'configure';
let importMs: number | null = null;
let executionStarted = 0;
let observerId = '';
let productionHandler: ((event: MessageEvent) => unknown) | null = null;

function fail() { post({ kind: 'failure', realmId, observerId }); }
self.onmessage = async (event: MessageEvent) => {
  try {
    if (stage === 'configure' && event.data.kind === 'configure') {
      stage = 'loading'; observerId = event.data.observerId;
      if (!event.data.empty) {
        // Catch the production completion locally. Never export payloads/closures.
        self.postMessage = ((message: any) => {
          if (stage !== 'executing' || message.messageType !== 'completed') { fail(); return; }
          const executionMs = performance.now() - executionStarted;
          stage = 'completed';
          post({ kind: 'completed', realmId, observerId, importMs, executionMs, response: message.response });
        }) as typeof self.postMessage;
        const before = performance.now();
        await import(event.data.entry);
        importMs = performance.now() - before;
        productionHandler = self.onmessage as (event: MessageEvent) => unknown;
        if (typeof productionHandler !== 'function' || productionHandler === diagnosticHandler) throw new Error('No production listener');
      }
      self.onmessage = diagnosticHandler;
      stage = 'ready'; post({ kind: 'ready', realmId, observerId, importMs }); return;
    }
    if (stage !== 'ready' || event.data.kind !== 'execute' || event.data.observerId !== observerId) throw new Error('Wrong diagnostic state');
    stage = 'executing'; executionStarted = performance.now();
    if (productionHandler === null) {
      stage = 'completed'; post({ kind: 'completed', realmId, observerId, importMs: null,
        executionMs: performance.now() - executionStarted, response: { ok: true, result: 1 } }); return;
    }
    productionHandler.call(self, { data: { messageType: 'execute', request: event.data.request as ScriptWorkerRequest } } as MessageEvent);
  } catch { fail(); }
};
const diagnosticHandler = self.onmessage;
post({ kind: 'entered', realmId });
