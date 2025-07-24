import { z } from 'zod';
import { WorkflowDefinition, StepDefinition, TriggerDefinition } from './types';

export const WorkflowDefinitionSchema = z.object({
  id: z.string().min(1, 'Workflow ID cannot be empty'),
  name: z.string().optional(),
  description: z.string().optional(),
  tags: z.array(z.string()).optional(),
  services: z.array(z.any()).optional(),
  hooks: z
    .object({
      onSuccess: z.function().optional(),
      onFailure: z.function().optional(),
    })
    .optional(),
  timeout: z.union([z.string(), z.number()]).optional(),
  concurrency: z.number().optional(),
  rateLimit: z
    .object({
      count: z.number(),
      per: z.string(),
    })
    .optional(),
  queue: z.string().optional(),
  version: z.string().optional(),
  secrets: z.object({}).optional(),
  steps: z
    .array(
      z.object({
        id: z.string().min(1, 'Step ID cannot be empty'),
        name: z.string().min(1, 'Step name cannot be empty'),
        handler: z.function(),
        type: z.enum(['step', 'action']),
        options: z
          .object({
            timeout: z.union([z.string(), z.number()]).optional(),
            retry: z
              .object({
                attempts: z.number(),
                backoff: z.object({
                  strategy: z.enum(['exponential', 'fixed']),
                  delay: z.union([z.string(), z.number()]),
                }),
              })
              .optional(),
            cache: z
              .object({
                key: z.function(),
                ttl: z.string(),
              })
              .optional(),
            delay: z.union([z.string(), z.number()]).optional(),
          })
          .optional(),
      })
    )
    .min(1, 'Workflow must have at least one step'),
  triggers: z.array(
    z.union([
      z.object({
        type: z.literal('webhook'),
        path: z.string().min(1, 'Webhook path cannot be empty'),
        options: z
          .object({
            method: z.enum(['POST', 'GET', 'PUT', 'DELETE']).optional(),
            schema: z.any().optional(),
            idempotencyKey: z.function().optional(),
            parseRawBody: z.boolean().optional(),
            headers: z
              .object({
                required: z.record(z.string(), z.string()).optional(),
                validate: z.function().optional(),
              })
              .optional(),
          })
          .optional(),
      }),
      z.object({
        type: z.literal('schedule'),
        cron_expression: z.string().min(1, 'Cron expression cannot be empty'),
      }),
      z.object({ type: z.literal('manual') }),
    ])
  ),
  created_at: z.date(),
  updated_at: z.date(),
});

export function validateWorkflow(workflow: WorkflowDefinition): void {
  try {
    WorkflowDefinitionSchema.parse(workflow);
  } catch (error) {
    if (error instanceof z.ZodError) {
      const errors = error.errors
        .map(e => `${e.path.join('.')}: ${e.message}`)
        .join(', ');
      throw new Error(`Workflow validation failed: ${errors}`);
    }
    throw error;
  }
}

export function validateStep(step: StepDefinition): void {
  if (!step.id || step.id.trim() === '') {
    throw new Error('Step ID cannot be empty');
  }
  if (!step.name || step.name.trim() === '') {
    throw new Error('Step name cannot be empty');
  }
  if (typeof step.handler !== 'function') {
    throw new Error('Step handler must be a function');
  }
}

export function validateTrigger(trigger: TriggerDefinition): void {
  if (
    trigger.type === 'webhook' &&
    (!trigger.path || trigger.path.trim() === '')
  ) {
    throw new Error('Webhook path cannot be empty');
  }
  if (
    trigger.type === 'schedule' &&
    (!trigger.cron_expression || trigger.cron_expression.trim() === '')
  ) {
    throw new Error('Cron expression cannot be empty');
  }
}
