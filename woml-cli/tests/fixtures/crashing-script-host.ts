import { encodeFrame } from '../../src/script-host/framing';

const ready = {
  protocol: 'woml.script-host',
  protocolVersion: Number(process.env.WOML_SCRIPT_HOST_PROTOCOL_VERSION ?? '3'),
  messageType: 'ready',
  hostInstanceId: 'host_r3_crash_fixture',
};

await new Promise<void>((resolve, reject) => {
  process.stdout.write(encodeFrame(ready), error => {
    if (error === null || error === undefined) resolve();
    else reject(error);
  });
});

await Bun.stdin.stream().getReader().read();
process.exit(17);
