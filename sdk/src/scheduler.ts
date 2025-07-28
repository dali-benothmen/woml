import { cronflow } from './cronflow';
import * as cron from 'node-cron';

interface ScheduledWorkflow {
  workflowId: string;
  cronExpression: string;
  task: cron.ScheduledTask;
  lastRun?: Date;
  nextRun?: Date;
}

class NodeScheduler {
  private scheduledWorkflows: Map<string, ScheduledWorkflow> = new Map();
  private isRunning: boolean = false;

  /**
   * Start the scheduler
   */
  start(): void {
    if (this.isRunning) {
      console.log(
        'ℹ️  Scheduler is already running (ignoring duplicate start call)'
      );
      return;
    }

    this.isRunning = true;
    console.log('🚀 Starting Node.js scheduler...');
  }

  /**
   * Stop the scheduler
   */
  stop(): void {
    if (!this.isRunning) {
      console.log('⚠️  Scheduler is not running');
      return;
    }

    this.isRunning = false;

    // Stop all scheduled tasks
    for (const [workflowId, scheduledWorkflow] of this.scheduledWorkflows) {
      scheduledWorkflow.task.stop();
      console.log(`⏹️  Stopped scheduled workflow: ${workflowId}`);
    }

    this.scheduledWorkflows.clear();
    console.log('🛑 Node.js scheduler stopped');
  }

  /**
   * Schedule a workflow to run based on a cron expression
   */
  scheduleWorkflow(workflowId: string, cronExpression: string): void {
    // Ensure scheduler is running
    if (!this.isRunning) {
      console.log('🔄 Starting scheduler for workflow scheduling...');
      this.start();
    }

    // Validate cron expression
    if (!cron.validate(cronExpression)) {
      throw new Error(`Invalid cron expression: ${cronExpression}`);
    }

    // Stop existing task if it exists
    const existing = this.scheduledWorkflows.get(workflowId);
    if (existing) {
      existing.task.stop();
      console.log(`🔄 Updating existing schedule for workflow: ${workflowId}`);
    }

    // Create the scheduled task
    const task = cron.schedule(
      cronExpression,
      async () => {
        try {
          console.log(
            `⏰ Executing scheduled workflow: ${workflowId} (cron: ${cronExpression})`
          );

          const scheduledWorkflow = this.scheduledWorkflows.get(workflowId);
          if (scheduledWorkflow) {
            scheduledWorkflow.lastRun = new Date();
            scheduledWorkflow.nextRun = this.getNextRunTime(cronExpression);
          }

          // Trigger the workflow with a scheduled payload
          const payload = {
            trigger_type: 'schedule',
            cron_expression: cronExpression,
            scheduled_at: new Date().toISOString(),
            workflow_id: workflowId,
          };

          const runId = await cronflow.trigger(workflowId, payload);
          console.log(
            `✅ Scheduled workflow ${workflowId} executed successfully with run ID: ${runId}`
          );
        } catch (error) {
          console.error(
            `❌ Failed to execute scheduled workflow ${workflowId}:`,
            error
          );
        }
      },
      {
        timezone: 'UTC',
      }
    );

    // Store the scheduled workflow
    this.scheduledWorkflows.set(workflowId, {
      workflowId,
      cronExpression,
      task,
      lastRun: undefined,
      nextRun: this.getNextRunTime(cronExpression),
    });

    console.log(
      `✅ Scheduled workflow ${workflowId} with cron: ${cronExpression}`
    );
    console.log(
      `   Next run: ${this.getNextRunTime(cronExpression)?.toISOString()}`
    );
  }

  /**
   * Unschedule a workflow
   */
  unscheduleWorkflow(workflowId: string): boolean {
    const scheduledWorkflow = this.scheduledWorkflows.get(workflowId);
    if (!scheduledWorkflow) {
      console.log(`⚠️  No scheduled workflow found with ID: ${workflowId}`);
      return false;
    }

    scheduledWorkflow.task.stop();
    this.scheduledWorkflows.delete(workflowId);
    console.log(`⏹️  Unscheduled workflow: ${workflowId}`);
    return true;
  }

  /**
   * Get all scheduled workflows
   */
  getScheduledWorkflows(): Array<{
    workflowId: string;
    cronExpression: string;
    lastRun?: Date;
    nextRun?: Date;
  }> {
    return Array.from(this.scheduledWorkflows.values()).map(sw => ({
      workflowId: sw.workflowId,
      cronExpression: sw.cronExpression,
      lastRun: sw.lastRun,
      nextRun: sw.nextRun,
    }));
  }

  /**
   * Get scheduled workflow by ID
   */
  getScheduledWorkflow(workflowId: string):
    | {
        workflowId: string;
        cronExpression: string;
        lastRun?: Date;
        nextRun?: Date;
      }
    | undefined {
    const scheduledWorkflow = this.scheduledWorkflows.get(workflowId);
    if (!scheduledWorkflow) {
      return undefined;
    }

    return {
      workflowId: scheduledWorkflow.workflowId,
      cronExpression: scheduledWorkflow.cronExpression,
      lastRun: scheduledWorkflow.lastRun,
      nextRun: scheduledWorkflow.nextRun,
    };
  }

  /**
   * Check if a workflow is scheduled
   */
  isScheduled(workflowId: string): boolean {
    return this.scheduledWorkflows.has(workflowId);
  }

  /**
   * Check if the scheduler is running
   */
  getRunningStatus(): boolean {
    return this.isRunning;
  }

  /**
   * Get scheduler status
   */
  getStatus(): {
    isRunning: boolean;
    scheduledWorkflowsCount: number;
    scheduledWorkflows: Array<{
      workflowId: string;
      cronExpression: string;
      lastRun?: Date;
      nextRun?: Date;
    }>;
  } {
    return {
      isRunning: this.isRunning,
      scheduledWorkflowsCount: this.scheduledWorkflows.size,
      scheduledWorkflows: this.getScheduledWorkflows(),
    };
  }

  /**
   * Calculate the next run time for a cron expression
   */
  private getNextRunTime(cronExpression: string): Date | undefined {
    try {
      // Use a simple approach to calculate next run time
      // For now, we'll just add 1 minute to current time as a placeholder
      // In a real implementation, you might want to use a more sophisticated cron parser
      const now = new Date();
      const nextRun = new Date(now.getTime() + 60000); // Add 1 minute
      return nextRun;
    } catch (error) {
      console.warn(
        `⚠️  Could not calculate next run time for cron: ${cronExpression}`
      );
      return undefined;
    }
  }
}

// Create a singleton instance
const scheduler = new NodeScheduler();

export { scheduler, NodeScheduler };
