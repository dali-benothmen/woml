import * as http from 'http';
import * as url from 'url';
import { loadCoreModule } from '../utils/core-resolver';
import { WorkflowDefinition } from '../workflow/types';

const { core } = loadCoreModule();

export interface WebhookServerConfig {
  host?: string;
  port?: number;
  maxConnections?: number;
}

export interface WebhookServer {
  server: http.Server;
  host: string;
  port: number;
}

export function createWebhookServer(
  config: WebhookServerConfig,
  getCurrentState: () => any,
  setState: (newState: any) => void,
  trigger: (workflowId: string, payload: any) => Promise<string>
): WebhookServer {
  const webhookConfig = {
    host: config.host || '127.0.0.1',
    port: config.port || 3000,
    max_connections: config.maxConnections || 1000,
  };

  if (webhookConfig.host !== '127.0.0.1') {
    process.env.CRONFLOW_WEBHOOK_HOST = webhookConfig.host;
  }
  if (webhookConfig.port !== 3000) {
    process.env.CRONFLOW_WEBHOOK_PORT = webhookConfig.port.toString();
  }
  if (webhookConfig.max_connections !== 1000) {
    process.env.CRONFLOW_WEBHOOK_MAX_CONNECTIONS =
      webhookConfig.max_connections.toString();
  }

  const currentState = getCurrentState();
  const result = core.startWebhookServer(currentState.dbPath);
  if (!result.success) {
    throw new Error(`Failed to start webhook server: ${result.message}`);
  }

  const server = http.createServer(async (req, res) => {
    try {
      const parsedUrl = url.parse(req.url || '', true);
      const path = parsedUrl.pathname || '';

      if (path === '/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(
          JSON.stringify({
            status: 'healthy',
            service: 'node-cronflow-webhook-server',
            timestamp: new Date().toISOString(),
          })
        );
        return;
      }

      // Match webhook path directly (no /webhook/ prefix required)
      const workflow = Array.from(currentState.workflows.values()).find(
        (w: any) =>
          w.triggers.some((t: any) => t.type === 'webhook' && t.path === path)
      ) as WorkflowDefinition | undefined;

      if (workflow) {
        const webhookTrigger = workflow.triggers.find(
          (t: any) => t.type === 'webhook' && t.path === path
        ) as { type: 'webhook'; path: string; options?: any } | undefined;

        const headers: Record<string, string> = {};
        for (const [key, value] of Object.entries(req.headers)) {
          if (Array.isArray(value)) {
            headers[key.toLowerCase()] = value[0];
          } else if (value) {
            headers[key.toLowerCase()] = value;
          }
        }

        if (webhookTrigger?.options?.headers) {
          const headerConfig = webhookTrigger.options.headers;

          if (headerConfig.required) {
            for (const [requiredHeader, expectedValue] of Object.entries(
              headerConfig.required
            )) {
              const actualValue = headers[requiredHeader.toLowerCase()];
              if (!actualValue) {
                res.writeHead(400, {
                  'Content-Type': 'application/json',
                });
                res.end(
                  JSON.stringify({
                    error: `Missing required header: ${requiredHeader}`,
                    required_headers: headerConfig.required,
                  })
                );
                return;
              }
              if (expectedValue && actualValue !== expectedValue) {
                res.writeHead(400, {
                  'Content-Type': 'application/json',
                });
                res.end(
                  JSON.stringify({
                    error: `Invalid header value for ${requiredHeader}: expected ${expectedValue}, got ${actualValue}`,
                    required_headers: headerConfig.required,
                  })
                );
                return;
              }
            }
          }

          if (headerConfig.validate) {
            const validationResult = headerConfig.validate(headers);
            if (validationResult !== true) {
              const errorMessage =
                typeof validationResult === 'string'
                  ? validationResult
                  : 'Header validation failed';
              res.writeHead(400, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({ error: errorMessage }));
              return;
            }
          }
        }

        let body = '';
        req.on('data', chunk => {
          body += chunk.toString();
        });

        req.on('end', async () => {
          try {
            const payload = body ? JSON.parse(body) : {};

            if (webhookTrigger?.options?.schema) {
              try {
                webhookTrigger.options.schema.parse(payload);
              } catch (schemaError: any) {
                res.writeHead(400, {
                  'Content-Type': 'application/json',
                });
                res.end(
                  JSON.stringify({
                    status: 'error',
                    message: 'Payload validation failed',
                    error: schemaError.message,
                    workflow_triggered: false,
                  })
                );
                return;
              }
            }

            const runId = await trigger(workflow.id, payload);

            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(
              JSON.stringify({
                status: 'success',
                message: 'Webhook processed successfully',
                workflow_triggered: true,
                run_id: runId,
              })
            );
          } catch (error) {
            res.writeHead(500, { 'Content-Type': 'application/json' });
            res.end(
              JSON.stringify({
                status: 'error',
                message:
                  error instanceof Error ? error.message : 'Unknown error',
                workflow_triggered: false,
              })
            );
          }
        });

        return;
      }

      // No matching webhook found
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'Not found' }));
    } catch (error) {
      res.writeHead(500, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'Internal server error' }));
    }
  });

  server.listen(webhookConfig.port, webhookConfig.host, () => {
    setState({ webhookServer: server });
  });

  setState({ webhookServer: server });

  return {
    server,
    host: webhookConfig.host,
    port: webhookConfig.port,
  };
}
