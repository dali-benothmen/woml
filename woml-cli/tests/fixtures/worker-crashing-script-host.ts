import {
  FrameDecoder,
  SerializedFrameWriter,
} from '../../src/script-host/framing';
import { ScriptHost } from '../../src/script-host/host';
import type { ReadyMessage } from '../../src/script-host/types';

async function writeStdout(frame: Uint8Array): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    process.stdout.write(frame, error => {
      if (error === null || error === undefined) resolve();
      else reject(error);
    });
  });
}

const decoder = new FrameDecoder();
const writer = new SerializedFrameWriter(writeStdout);
const protocolVersion = Number(
  process.env.WOML_SCRIPT_HOST_PROTOCOL_VERSION ?? '3'
) as ReadyMessage['protocolVersion'];
const host = new ScriptHost({
  workerUrl: new URL('./missing-script-worker.ts', import.meta.url),
  protocolVersion,
  send: message => writer.send(message),
});
const ready: ReadyMessage = {
  protocol: 'woml.script-host',
  protocolVersion,
  messageType: 'ready',
  hostInstanceId: 'host_ri7_worker_crash',
};

await writer.send(ready);
const reader = Bun.stdin.stream().getReader();
while (true) {
  const { done, value } = await reader.read();
  if (done) break;
  for (const message of decoder.push(value)) host.accept(message);
}
decoder.finish();
await host.drain();
await writer.drain();
