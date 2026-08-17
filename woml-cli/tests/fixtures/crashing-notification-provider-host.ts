import { randomUUID } from 'node:crypto';

import { SerializedFrameWriter } from '../../src/script-host/framing';
import {
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type ReadyMessage,
} from '../../src/notification-provider';

const writer = new SerializedFrameWriter(
  frame =>
    new Promise<void>((resolve, reject) => {
      process.stdout.write(frame, error => {
        if (error === null || error === undefined) resolve();
        else reject(error);
      });
    })
);
const ready: ReadyMessage = {
  protocol: NOTIFICATION_PROVIDER_PROTOCOL,
  protocolVersion: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  messageType: 'ready',
  hostInstanceId: `crashing_notification_host_${randomUUID()}`,
  providers: ['slack'],
};
await writer.send(ready);
await Bun.stdin.stream().getReader().read();
process.exit(17);
