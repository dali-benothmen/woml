#!/usr/bin/env bun

const [command, channel = '#rehearsal'] = Bun.argv.slice(2);
if (command !== 'notify') {
  console.error('Usage: slack.ts notify [channel]');
  process.exit(2);
}
console.log(JSON.stringify({
  delivered: true,
  provider: 'fake-slack',
  channel,
  messageId: 'rehearsal-message-42',
}));
