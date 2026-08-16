import { describe, expect, test } from 'bun:test';

import { assertNotificationInvocation } from '../src/notification-provider/protocol';
import { SharedWhatsAppTransport, WhatsAppTransportError } from '../src/whatsapp';

describe('ACP7 WhatsApp runtime', () => {
  test('sends approved templates with stable approval capability buttons', async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const transport = new SharedWhatsAppTransport({
      apiBase: 'https://meta.test/v23.0',
      fetch: (async (input, init) => {
        calls.push({ url: String(input), init });
        return new Response(JSON.stringify({ messages: [{ id: 'wamid.test-42' }] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }) as typeof fetch,
    });
    const identity = await transport.sendTemplate({
      accessToken: 'secret-token',
      phoneNumberId: '123456789012345',
      conversationId: '15551234567',
      templateName: 'woml_approval_v1',
      language: 'en_US',
      parameters: ['Approval', 'Description', 'workflow-id', 'deadline'],
      decisionCapability: 'ncap_1234567890abcdef.1234567890abcdef1234567890abcdef',
    });
    expect(identity).toEqual({
      provider: 'whatsapp',
      accountId: '123456789012345',
      conversationId: '15551234567',
      messageId: 'wamid.test-42',
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe('https://meta.test/v23.0/123456789012345/messages');
    expect(calls[0]!.init?.headers).toEqual({
      authorization: 'Bearer secret-token',
      'content-type': 'application/json',
    });
    const body = JSON.parse(String(calls[0]!.init?.body));
    expect(body.template.name).toBe('woml_approval_v1');
    expect(body.template.components[1].parameters[0].payload).toStartWith('woml_approve:ncap_');
    expect(body.template.components[2].parameters[0].payload).toStartWith('woml_reject:ncap_');
    await transport.close();
  });

  test('maps Meta rate limits without exposing response details', async () => {
    const transport = new SharedWhatsAppTransport({
      fetch: (async () => new Response(
        JSON.stringify({ error: { code: 4, message: 'contains provider detail' } }),
        { status: 429 }
      )) as unknown as typeof fetch,
    });
    try {
      await transport.sendTemplate({
        accessToken: 'secret-token',
        phoneNumberId: '123456789012345',
        conversationId: '15551234567',
        templateName: 'notice',
        language: 'en_US',
        parameters: ['hello'],
      });
      throw new Error('expected a WhatsApp rate limit');
    } catch (error) {
      expect(error).toBeInstanceOf(WhatsAppTransportError);
      expect((error as WhatsAppTransportError).failure).toMatchObject({
        kind: 'rate_limited',
        code: 'WOML_WHATSAPP_RATE_LIMITED',
        retryable: true,
      });
      expect((error as Error).message).not.toContain('provider detail');
    }
  });

  test('accepts frozen v1 approval and v2 lifecycle invocation envelopes', () => {
    const credentials = {
      accessToken: { kind: 'secretReference', name: 'WHATSAPP_ACCESS_TOKEN' },
      phoneNumberId: '123456789012345',
    } as const;
    expect(() => assertNotificationInvocation({
      protocol: 'woml.notification-provider-host',
      protocolVersion: 1,
      messageType: 'deliver',
      invocationId: 'notification-whatsapp',
      runId: 'run_whatsapp',
      approvalId: 'review',
      requestId: 'aprreq_whatsapp',
      deliveryId: 'review:notify:0:recipient:0',
      provider: 'whatsapp',
      destination: '15551234567',
      idempotencyKey: `sha256:${'a'.repeat(64)}`,
      credentials,
      decisionCapability: `ncap_${'a'.repeat(16)}.${'b'.repeat(32)}`,
      templateName: 'woml_approval_v1',
      language: 'en_US',
      message: {
        workflowId: 'whatsapp-test',
        approvalName: 'Approve request',
      },
    })).not.toThrow();
    expect(() => assertNotificationInvocation({
      protocol: 'woml.notification-provider-host',
      protocolVersion: 2,
      messageType: 'deliver',
      mode: 'informational',
      invocationId: 'notification-whatsapp-lifecycle',
      runId: 'run_whatsapp',
      hookInvocationId: `sha256:${'b'.repeat(64)}`,
      actionId: 'lifecycle:run_complete:action:0',
      deliveryId: 'lifecycle:run_complete:action:0:provider:0:recipient:0',
      provider: 'whatsapp',
      destination: '15551234567',
      idempotencyKey: `sha256:${'c'.repeat(64)}`,
      credentials,
      templateName: 'workflow_completed',
      language: 'en_US',
      message: 'Workflow completed.',
    })).not.toThrow();
  });
});
