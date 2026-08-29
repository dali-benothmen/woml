import type {
  ExecutionProgressV1,
  IntervalProgressV1,
  ScheduleProgressV1,
  TriggerProgressV1,
  WorkflowCallProgressV1,
} from './rust-executor';
import {
  renderPresentationWarning,
  renderRunAdmission,
  renderRunNotice,
  renderRunPresentation,
  renderWorkflowStartup,
  sanitizeTerminalText,
  type PresentationRenderOptions,
  type RunPresentationV1,
  type TriggerPresentationType,
  type WorkflowPresentationV1,
} from './terminal-presentation';
import { profileSync } from './performance-profiler';

const textEncoder = new TextEncoder();

interface PresentationOutput {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
}

export interface ForegroundPresentationOptions {
  readonly io: PresentationOutput;
  readonly render: PresentationRenderOptions;
  readonly verbose: boolean;
  readonly inspectRun: (runId: string) => RunPresentationV1;
}

function triggerType(handler: string): TriggerPresentationType {
  const type = handler.startsWith('trigger.')
    ? handler.slice('trigger.'.length)
    : handler;
  return ['manual', 'webhook', 'slack', 'schedule', 'interval', 'event'].includes(type)
    ? (type as TriggerPresentationType)
    : 'manual';
}

/**
 * Keeps terminal formatting out of runtime callbacks. Durable Rust projection
 * remains authoritative; this class only chooses when and where to render it.
 */
export class ForegroundPresentation {
  readonly #io: PresentationOutput;
  readonly #render: PresentationRenderOptions;
  readonly #verbose: boolean;
  readonly #inspectRun: (runId: string) => RunPresentationV1;
  readonly #settled = new Set<string>();
  readonly #admitted = new Set<string>();
  readonly #notices = new Set<string>();

  constructor(options: ForegroundPresentationOptions) {
    this.#io = options.io;
    this.#render = options.render;
    this.#verbose = options.verbose;
    this.#inspectRun = options.inspectRun;
  }

  #write(text: string): void {
    profileSync('presentation', 'presentation.output_write', measurements => {
      if (measurements !== undefined) {
        measurements.counts.writes = 1;
        measurements.counts.json = this.#render.format === 'json' ? 1 : 0;
        measurements.bytes.output = textEncoder.encode(text).byteLength;
      }
      if (this.#render.format === 'json') this.#io.stdout(text);
      else this.#io.stderr(text);
    });
  }

  startup(workflow: WorkflowPresentationV1): void {
    profileSync('presentation', 'presentation.handle_startup', measurements => {
      if (measurements !== undefined) measurements.counts.workflows = 1;
      const rendered = profileSync(
        'presentation',
        'presentation.render_startup',
        renderMeasurements => {
          const text = renderWorkflowStartup(workflow, this.#render);
          if (renderMeasurements !== undefined) {
            renderMeasurements.bytes.output = textEncoder.encode(text).byteLength;
          }
          return text;
        }
      );
      this.#write(rendered);
    });
  }

  verbose(message: string): void {
    if (!this.#verbose) return;
    this.#io.stderr(`[woml:verbose] ${sanitizeTerminalText(message).replaceAll('\n', ' ')}\n`);
  }

  warning(code: string, message: string): void {
    if (this.#render.format === 'json') {
      this.#io.stderr(
        `Warning [${sanitizeTerminalText(code)}]: ${sanitizeTerminalText(message).replaceAll('\n', ' ')}\n`
      );
      return;
    }
    this.#write(renderPresentationWarning(code, message, this.#render));
  }

  trigger(progress: TriggerProgressV1): void {
    profileSync('presentation', 'presentation.handle_progress', measurements => {
      if (measurements !== undefined) {
        measurements.counts.progress_events = 1;
        if ('runId' in progress) measurements.identity.runId = progress.runId;
      }
      this.#trigger(progress);
    });
  }

  #trigger(progress: TriggerProgressV1): void {
    if (progress.type === 'occurrence_accepted') {
      if (progress.duplicate) {
        this.warning(
          'WOML_TRIGGER_DUPLICATE',
          `Duplicate ${progress.triggerHandler} occurrence reused run ${progress.runId}.`
        );
        return;
      }
      if (!this.#admitted.has(progress.runId)) {
        this.#admitted.add(progress.runId);
        if (this.#render.format === 'json') {
          this.#snapshot(progress.runId);
        } else {
          this.#write(renderRunAdmission({
            runId: progress.runId,
            admittedAt: progress.occurredAt,
            workflowId: progress.workflowId,
            triggerId: progress.triggerId,
            triggerType: triggerType(progress.triggerHandler),
          }, this.#render));
        }
      }
      return;
    }
    if (progress.type === 'occurrence_rejected') {
      this.warning(progress.code, progress.message);
      return;
    }
    if (progress.type === 'run_terminal') {
      if (this.#settled.has(progress.runId)) return;
      try {
        profileSync('presentation', 'presentation.present_terminal_result', measurements => {
          if (measurements !== undefined) measurements.identity.runId = progress.runId;
          const presentation = profileSync(
            'presentation',
            'presentation.inspect_durable_run',
            inspectMeasurements => {
              if (inspectMeasurements !== undefined) {
                inspectMeasurements.identity.runId = progress.runId;
              }
              return this.#inspectRun(progress.runId);
            }
          );
          this.#settled.add(progress.runId);
          // One write is deliberate: concurrent runs never interleave rows.
          const rendered = profileSync(
            'presentation',
            'presentation.render_final',
            renderMeasurements => {
              if (renderMeasurements !== undefined) {
                renderMeasurements.identity.runId = progress.runId;
                renderMeasurements.counts.steps = presentation.steps.length;
              }
              const text = renderRunPresentation(presentation, this.#render);
              if (renderMeasurements !== undefined) {
                renderMeasurements.bytes.output = textEncoder.encode(text).byteLength;
              }
              return text;
            }
          );
          this.#write(this.#render.format === 'json' ? rendered : `\n${rendered}`);
        });
      } catch (error) {
        this.warning(
          'WOML_RUN_PRESENTATION_UNAVAILABLE',
          `Run ${progress.runId} settled, but its presentation is temporarily unavailable: ${error instanceof Error ? error.message : String(error)}`
        );
      }
      return;
    }
    this.verbose(progress.type === 'ready'
      ? `Runtime registered ${progress.registrationCount} trigger(s).`
      : `Run ${progress.runId} started.`);
  }

  execution(progress: ExecutionProgressV1): void {
    if ('profile' in progress && progress.profile === 'woml.runtime-policy-progress/v1') {
      if (progress.phase === 'queued') {
        const reason = progress.waitingFor?.replaceAll('_', ' ') ?? 'capacity';
        this.#notice(progress.runId, 'queued', `Waiting for ${reason}${progress.eligibleAt === undefined ? '' : ` until ${progress.eligibleAt}`}`);
      } else if (progress.phase === 'timed_out') {
        this.warning(progress.code ?? 'WOML_WORKFLOW_TIMED_OUT', `Run ${progress.runId} exceeded its workflow timeout.`);
      } else {
        this.verbose(`Run ${progress.runId} policy phase ${progress.phase}.`);
      }
      return;
    }
    if ('profile' in progress) {
      if (progress.phase === 'run_finalizing') {
        this.#notice(progress.runId, 'finalizing', 'Completing lifecycle hooks');
      } else {
        this.verbose(`Lifecycle ${progress.hookId} ${progress.phase} for run ${progress.runId}.`);
      }
      return;
    }
    if (progress.type === 'for_each_progress') {
      const completed = progress.succeeded + progress.failed + progress.skipped;
      const message = progress.status === 'running'
        ? `For each ${progress.forEachId} · ${completed}/${progress.total} completed · ${progress.active} active`
        : `For each ${progress.forEachId} · ${progress.succeeded} succeeded · ${progress.failed} failed · ${progress.skipped} skipped`;
      if (this.#render.format === 'json') this.#snapshot(progress.runId);
      else this.#notice(
        progress.runId,
        progress.status === 'running' ? 'running' : progress.status,
        message
      );
      return;
    }
    if (progress.type === 'step_retry_scheduled') {
      this.#notice(
        progress.runId,
        'retrying',
        `Step ${progress.nodeId} will retry (${progress.nextAttempt}/${progress.maxAttempts})`
      );
      return;
    }
    this.verbose(
      `Step ${progress.nodeId} ${progress.type === 'step_attempt_failed' ? 'failed' : 'succeeded'} on attempt ${progress.attempt}/${progress.maxAttempts}.`
    );
  }

  schedule(progress: ScheduleProgressV1): void {
    if (progress.type === 'scheduler_error') this.warning(progress.code, progress.message);
    else this.verbose(`Schedule ${progress.triggerId} next due at ${progress.nextScheduledAt}.`);
  }

  interval(progress: IntervalProgressV1): void {
    if (progress.type === 'scheduler_error') this.warning(progress.code, progress.message);
    else this.verbose(`Interval ${progress.triggerId} next due at ${progress.nextScheduledAt}.`);
  }

  workflowCall(progress: WorkflowCallProgressV1): void {
    if (progress.type === 'call_rejected') this.warning(progress.code, progress.message);
    else if (progress.type === 'call_admitted') {
      this.verbose(`Run ${progress.parentRunId} admitted child ${progress.childRunId} (${progress.targetWorkflowId}).`);
    } else {
      this.verbose(`Child run ${progress.childRunId} ${progress.status}.`);
    }
  }

  #notice(
    runId: string,
    status: 'queued' | 'running' | 'waiting' | 'retrying' | 'finalizing' | 'succeeded' | 'failed' | 'cancelled',
    message: string
  ): void {
    const identity = `${runId}:${status}:${message}`;
    if (this.#notices.has(identity)) return;
    this.#notices.add(identity);
    if (this.#render.format === 'json') this.#snapshot(runId);
    else this.#write(renderRunNotice(runId, status, message, this.#render));
  }

  #snapshot(runId: string): void {
    try {
      const presentation = profileSync(
        'presentation',
        'presentation.inspect_durable_run',
        measurements => {
          if (measurements !== undefined) measurements.identity.runId = runId;
          return this.#inspectRun(runId);
        }
      );
      const rendered = profileSync(
        'presentation',
        'presentation.render_snapshot',
        measurements => {
          if (measurements !== undefined) {
            measurements.identity.runId = runId;
            measurements.counts.steps = presentation.steps.length;
          }
          const text = renderRunPresentation(presentation, this.#render);
          if (measurements !== undefined) {
            measurements.bytes.output = textEncoder.encode(text).byteLength;
          }
          return text;
        }
      );
      this.#write(rendered);
    } catch (error) {
      this.warning(
        'WOML_RUN_PRESENTATION_UNAVAILABLE',
        `Run ${runId} changed state, but its presentation is temporarily unavailable: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }
}
