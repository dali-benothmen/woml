# Intelligent Error Management

Smart error clustering, impact analysis, and automated response

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Types
interface ErrorEvent {
  id: string;
  timestamp: Date;
  level: 'error' | 'warning' | 'critical';
  message: string;
  stackTrace: string;
  service: string;
  version: string;
  userId?: string;
  userTier?: 'free' | 'premium' | 'enterprise';
  metadata: Record<string, any>;
}

interface ErrorCluster {
  id: string;
  pattern: string;
  count: number;
  firstSeen: Date;
  lastSeen: Date;
  services: string[];
  affectedUsers: {
    total: number;
    premium: number;
    enterprise: number;
  };
  severity: 'low' | 'medium' | 'high' | 'critical';
}

interface SystemHealth {
  cpu: number;
  memory: number;
  diskSpace: number;
  responseTime: number;
  errorRate: number;
  status: 'healthy' | 'degraded' | 'critical';
}

interface HistoricalMatch {
  clusterId: string;
  similarity: number;
  resolution: string;
  timeToResolve: number;
  wasAutomated: boolean;
  date: Date;
}

interface AutomatedActionResult {
  action: string;
  success: boolean;
  error?: string;
}

// Mock data storage
const errorClusters: Map<string, ErrorCluster> = new Map();
const errorHistory: ErrorEvent[] = [];
const deploymentHistory: Array<{
  id: string;
  timestamp: Date;
  service: string;
  version: string;
  rollbackable: boolean;
}> = [];

// Schema for error webhook
const errorEventSchema = z.object({
  level: z.enum(['error', 'warning', 'critical']),
  message: z.string(),
  stackTrace: z.string(),
  service: z.string(),
  version: z.string(),
  userId: z.string().optional(),
  userTier: z.enum(['free', 'premium', 'enterprise']).optional(),
  metadata: z.record(z.any()).optional(),
});

// Mock functions
async function getSystemHealth(): Promise<SystemHealth> {
  return {
    cpu: 30 + Math.random() * 40,
    memory: 50 + Math.random() * 30,
    diskSpace: 20 + Math.random() * 50,
    responseTime: 100 + Math.random() * 200,
    errorRate: Math.random() * 5,
    status: Math.random() > 0.8 ? 'degraded' : 'healthy',
  };
}

async function findSimilarErrors(
  errorMessage: string,
  stackTrace: string
): Promise<HistoricalMatch[]> {
  // Mock similarity search
  const matches: HistoricalMatch[] = [];

  if (Math.random() > 0.7) {
    matches.push({
      clusterId: 'cluster_db_timeout',
      similarity: 0.85,
      resolution: 'Database connection pool increased',
      timeToResolve: 45, // minutes
      wasAutomated: false,
      date: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000),
    });
  }

  if (Math.random() > 0.8) {
    matches.push({
      clusterId: 'cluster_memory_leak',
      similarity: 0.72,
      resolution: 'Service restart resolved issue',
      timeToResolve: 20,
      wasAutomated: true,
      date: new Date(Date.now() - 3 * 24 * 60 * 60 * 1000),
    });
  }

  return matches;
}

function clusterError(error: ErrorEvent): string {
  // Simple clustering logic - normalize error messages
  let pattern = error.message
    .replace(/\d+/g, 'N') // Replace numbers with N
    .replace(/[a-f0-9-]{36}/g, 'UUID') // Replace UUIDs
    .replace(/\/[a-zA-Z0-9\/]+/g, '/PATH') // Replace file paths
    .replace(/\b\w+@\w+\.\w+/g, 'EMAIL'); // Replace emails

  return `${error.service}:${pattern}`;
}

function calculateErrorImpact(
  cluster: ErrorCluster,
  systemHealth: SystemHealth
): {
  score: number;
  factors: string[];
  severity: 'low' | 'medium' | 'high' | 'critical';
} {
  let score = 0;
  const factors: string[] = [];

  // User impact
  if (cluster.affectedUsers.enterprise > 0) {
    score += 40;
    factors.push(
      `${cluster.affectedUsers.enterprise} enterprise users affected`
    );
  }

  if (cluster.affectedUsers.premium > 5) {
    score += 30;
    factors.push(`${cluster.affectedUsers.premium} premium users affected`);
  }

  // Frequency impact
  if (cluster.count > 100) {
    score += 25;
    factors.push(`High frequency: ${cluster.count} occurrences`);
  } else if (cluster.count > 20) {
    score += 15;
    factors.push(`Medium frequency: ${cluster.count} occurrences`);
  }

  // System health impact
  if (systemHealth.status === 'critical') {
    score += 30;
    factors.push('System in critical state');
  } else if (systemHealth.status === 'degraded') {
    score += 15;
    factors.push('System performance degraded');
  }

  // Service criticality
  const criticalServices = ['payment', 'auth', 'database'];
  if (cluster.services.some(s => criticalServices.includes(s))) {
    score += 20;
    factors.push('Critical service affected');
  }

  let severity: 'low' | 'medium' | 'high' | 'critical' = 'low';
  if (score >= 80) severity = 'critical';
  else if (score >= 60) severity = 'high';
  else if (score >= 30) severity = 'medium';

  return { score, factors, severity };
}

function generateDiagnosis(
  cluster: ErrorCluster,
  systemHealth: SystemHealth,
  historicalMatches: HistoricalMatch[]
): {
  diagnosis: string;
  confidence: number;
  suggestedActions: string[];
  automatedActions: string[];
} {
  const diagnosis: string[] = [];
  const suggestedActions: string[] = [];
  const automatedActions: string[] = [];
  let confidence = 0.5;

  // Historical analysis
  const bestMatch = historicalMatches.find(m => m.similarity > 0.8);
  if (bestMatch) {
    diagnosis.push(
      `Similar to previous issue from ${bestMatch.date.toDateString()}`
    );
    diagnosis.push(`Previous resolution: ${bestMatch.resolution}`);
    confidence += 0.3;

    if (bestMatch.wasAutomated) {
      automatedActions.push('Apply previous automated fix');
    } else {
      suggestedActions.push(bestMatch.resolution);
    }
  }

  // System health correlation
  if (systemHealth.memory > 85) {
    diagnosis.push('High memory usage detected');
    suggestedActions.push('Investigate memory leaks');
    automatedActions.push('Scale up memory resources');
    confidence += 0.2;
  }

  if (systemHealth.errorRate > 3) {
    diagnosis.push('Elevated error rate across system');
    suggestedActions.push('Check recent deployments');
  }

  // Pattern analysis
  if (
    cluster.pattern.includes('database') ||
    cluster.pattern.includes('timeout')
  ) {
    diagnosis.push('Database connectivity issue suspected');
    suggestedActions.push('Check database performance metrics');
    suggestedActions.push('Review connection pool settings');
    confidence += 0.15;
  }

  if (
    cluster.pattern.includes('memory') ||
    cluster.pattern.includes('OutOfMemory')
  ) {
    diagnosis.push('Memory-related issue detected');
    automatedActions.push('Restart affected services');
    confidence += 0.2;
  }

  // Recent deployment correlation
  const recentDeployment = deploymentHistory
    .filter(d => d.timestamp > new Date(Date.now() - 2 * 60 * 60 * 1000)) // Last 2 hours
    .find(d => cluster.services.includes(d.service));

  if (recentDeployment) {
    diagnosis.push(
      `Recent deployment detected: ${recentDeployment.service} v${recentDeployment.version}`
    );
    if (recentDeployment.rollbackable) {
      automatedActions.push(
        `Rollback ${recentDeployment.service} to previous version`
      );
    }
    suggestedActions.push('Review deployment changes');
    confidence += 0.25;
  }

  return {
    diagnosis: diagnosis.join('. '),
    confidence: Math.min(0.95, confidence),
    suggestedActions,
    automatedActions,
  };
}

async function executeAutomatedAction(
  action: string,
  context: any
): Promise<boolean> {
  console.log(`🤖 Executing automated action: ${action}`);

  // Mock automated actions
  if (action.includes('Scale up memory')) {
    console.log('   → Scaling up memory resources...');
    await new Promise(resolve => setTimeout(resolve, 2000));
    return true;
  }

  if (action.includes('Restart affected services')) {
    console.log('   → Restarting services...');
    await new Promise(resolve => setTimeout(resolve, 3000));
    return true;
  }

  if (action.includes('Rollback')) {
    console.log('   → Rolling back deployment...');
    await new Promise(resolve => setTimeout(resolve, 5000));
    return true;
  }

  return false;
}

// Main Error Management Workflow
const errorManagementWorkflow = cronflow.define({
  id: 'intelligent-error-management',
  name: 'Intelligent Error Management',
  description:
    'Smart error clustering, impact analysis, and automated response',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        console.log('🎯 Error management workflow completed');
      }
    },
    onFailure: (ctx, stepId) => {
      console.error(
        `❌ Error management failed at step ${stepId}:`,
        ctx.step_error
      );
    },
  },
});

// Main error ingestion webhook
errorManagementWorkflow
  .onWebhook('/webhooks/error-event', {
    schema: errorEventSchema,
  })
  .step('process-error-event', async ctx => {
    const errorData = ctx.payload;

    const errorEvent: ErrorEvent = {
      id: `error_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`,
      timestamp: new Date(),
      level: errorData.level,
      message: errorData.message,
      stackTrace: errorData.stackTrace,
      service: errorData.service,
      version: errorData.version,
      userId: errorData.userId,
      userTier: errorData.userTier,
      metadata: errorData.metadata || {},
    };

    // Store error
    errorHistory.push(errorEvent);

    console.log(
      `📥 New ${errorEvent.level} error from ${errorEvent.service}: ${errorEvent.message.substring(0, 100)}...`
    );

    return { errorEvent };
  })
  .step('cluster-error', async ctx => {
    const { errorEvent } = ctx.last;

    // Generate cluster pattern
    const pattern = clusterError(errorEvent);

    // Find or create cluster
    let cluster = errorClusters.get(pattern);

    if (!cluster) {
      cluster = {
        id: `cluster_${Date.now()}`,
        pattern,
        count: 1,
        firstSeen: errorEvent.timestamp,
        lastSeen: errorEvent.timestamp,
        services: [errorEvent.service],
        affectedUsers: {
          total: errorEvent.userId ? 1 : 0,
          premium: errorEvent.userTier === 'premium' ? 1 : 0,
          enterprise: errorEvent.userTier === 'enterprise' ? 1 : 0,
        },
        severity: 'low',
      };

      errorClusters.set(pattern, cluster);
      console.log(`🆕 New error cluster created: ${cluster.id}`);
    } else {
      // Update existing cluster
      cluster.count++;
      cluster.lastSeen = errorEvent.timestamp;

      if (!cluster.services.includes(errorEvent.service)) {
        cluster.services.push(errorEvent.service);
      }

      if (errorEvent.userId) {
        cluster.affectedUsers.total++;
        if (errorEvent.userTier === 'premium') cluster.affectedUsers.premium++;
        if (errorEvent.userTier === 'enterprise')
          cluster.affectedUsers.enterprise++;
      }

      console.log(
        `📊 Updated cluster ${cluster.id}: ${cluster.count} occurrences`
      );
    }

    return { errorEvent, cluster };
  })
  .parallel([
    // Get system health
    async ctx => {
      const systemHealth = await getSystemHealth();
      console.log(
        `🏥 System health: ${systemHealth.status} (CPU: ${systemHealth.cpu.toFixed(1)}%, Memory: ${systemHealth.memory.toFixed(1)}%)`
      );
      return { systemHealth };
    },
    // Find historical matches
    async ctx => {
      const { errorEvent } = ctx.last;
      const historicalMatches = await findSimilarErrors(
        errorEvent.message,
        errorEvent.stackTrace
      );
      console.log(`🔍 Found ${historicalMatches.length} historical matches`);
      return { historicalMatches };
    },
  ])
  .step('analyze-impact', async ctx => {
    const { cluster } = ctx.last;
    const { systemHealth } = ctx.last[0];

    const impact = calculateErrorImpact(cluster, systemHealth);

    // Update cluster severity
    cluster.severity = impact.severity;

    console.log(
      `📈 Impact analysis: ${impact.severity} severity (score: ${impact.score})`
    );
    impact.factors.forEach(factor => console.log(`   - ${factor}`));

    return { cluster, impact, systemHealth };
  })
  .step('generate-diagnosis', async ctx => {
    const { cluster, impact, systemHealth } = ctx.last;
    const { historicalMatches } = ctx.last[1];

    const diagnosis = generateDiagnosis(
      cluster,
      systemHealth,
      historicalMatches
    );

    console.log(
      `🧠 Diagnosis (${(diagnosis.confidence * 100).toFixed(1)}% confidence):`
    );
    console.log(`   ${diagnosis.diagnosis}`);

    if (diagnosis.suggestedActions.length > 0) {
      console.log(`💡 Suggested actions:`);
      diagnosis.suggestedActions.forEach(action =>
        console.log(`   - ${action}`)
      );
    }

    if (diagnosis.automatedActions.length > 0) {
      console.log(`🤖 Automated actions available:`);
      diagnosis.automatedActions.forEach(action =>
        console.log(`   - ${action}`)
      );
    }

    return { cluster, impact, diagnosis };
  })
  .if('should-auto-remediate', ctx => {
    const { impact, diagnosis } = ctx.last;
    return (
      impact.severity === 'critical' &&
      diagnosis.automatedActions.length > 0 &&
      diagnosis.confidence > 0.7
    );
  })
  .step('execute-automated-actions', async ctx => {
    const { diagnosis, cluster } = ctx.last;

    console.log(
      `🚀 Executing ${diagnosis.automatedActions.length} automated actions...`
    );

    const results: AutomatedActionResult[] = [];
    for (const action of diagnosis.automatedActions) {
      try {
        const success = await executeAutomatedAction(action, { cluster });
        results.push({ action, success });
      } catch (error) {
        console.error(`❌ Failed to execute action: ${action}`, error);
        results.push({
          action,
          success: false,
          error: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    }

    const successfulActions = results.filter(r => r.success).length;
    console.log(
      `✅ ${successfulActions}/${results.length} automated actions completed successfully`
    );

    return { automatedResults: results, successfulActions };
  })
  .endIf()
  .step('determine-escalation', async ctx => {
    const { impact, diagnosis, cluster } = ctx.last;
    const automatedResults = ctx.last.successfulActions || 0;

    let shouldEscalate = false;
    let escalationLevel = 'none';
    let escalationReason = '';

    // Escalation logic
    if (impact.severity === 'critical') {
      shouldEscalate = true;
      escalationLevel = 'immediate';
      escalationReason = 'Critical severity error cluster';
    } else if (
      impact.severity === 'high' &&
      cluster.affectedUsers.enterprise > 0
    ) {
      shouldEscalate = true;
      escalationLevel = 'urgent';
      escalationReason = 'Enterprise users affected';
    } else if (impact.severity === 'high' && diagnosis.confidence < 0.5) {
      shouldEscalate = true;
      escalationLevel = 'normal';
      escalationReason = 'High impact with uncertain diagnosis';
    } else if (
      automatedResults === 0 &&
      diagnosis.automatedActions.length > 0
    ) {
      shouldEscalate = true;
      escalationLevel = 'normal';
      escalationReason = 'Automated remediation failed';
    }

    if (shouldEscalate) {
      console.log(`🚨 Escalating to DevOps team: ${escalationLevel} priority`);
      console.log(`   Reason: ${escalationReason}`);
    } else {
      console.log(`✅ No escalation needed - handled automatically`);
    }

    return {
      shouldEscalate,
      escalationLevel,
      escalationReason,
      cluster,
      impact,
      diagnosis,
    };
  })
  .if('needs-escalation', ctx => ctx.last.shouldEscalate)
  .step('send-alert', async ctx => {
    const { escalationLevel, escalationReason, cluster, impact, diagnosis } =
      ctx.last;

    const alert = {
      title: `${escalationLevel.toUpperCase()}: Error Cluster Alert`,
      cluster: {
        id: cluster.id,
        pattern: cluster.pattern,
        count: cluster.count,
        services: cluster.services,
        severity: impact.severity,
      },
      impact: {
        score: impact.score,
        factors: impact.factors,
        affectedUsers: cluster.affectedUsers,
      },
      diagnosis: {
        summary: diagnosis.diagnosis,
        confidence: diagnosis.confidence,
        suggestedActions: diagnosis.suggestedActions,
      },
      escalationReason,
      timestamp: new Date().toISOString(),
    };

    // Mock sending alert (could be Slack, PagerDuty, email, etc.)
    console.log(
      `📧 Alert sent to DevOps team:`,
      JSON.stringify(alert, null, 2)
    );

    return { alertSent: true, alert };
  })
  .endIf()
  .action('log-completion', ctx => {
    const { cluster, impact } = ctx.last;
    console.log(
      `✅ Error management completed for cluster ${cluster.id} (${impact.severity} severity)`
    );
  });

// Cluster cleanup workflow (runs every hour)
const cleanupWorkflow = cronflow.define({
  id: 'error-cluster-cleanup',
  name: 'Error Cluster Cleanup',
  description: 'Clean up old error clusters and maintain system health',
});

cleanupWorkflow
  .onSchedule('0 * * * *') // Every hour
  .step('cleanup-old-clusters', async ctx => {
    const now = new Date();
    const cutoffTime = new Date(now.getTime() - 24 * 60 * 60 * 1000); // 24 hours ago

    let cleaned = 0;

    for (const [pattern, cluster] of errorClusters.entries()) {
      if (cluster.lastSeen < cutoffTime && cluster.severity === 'low') {
        errorClusters.delete(pattern);
        cleaned++;
      }
    }

    console.log(`🧹 Cleaned up ${cleaned} old error clusters`);
    console.log(`📊 Active clusters: ${errorClusters.size}`);

    return { cleanedClusters: cleaned, activeClusters: errorClusters.size };
  })
  .step('generate-health-report', async ctx => {
    const systemHealth = await getSystemHealth();

    const report = {
      timestamp: new Date().toISOString(),
      systemHealth,
      errorClusters: {
        total: errorClusters.size,
        critical: Array.from(errorClusters.values()).filter(
          c => c.severity === 'critical'
        ).length,
        high: Array.from(errorClusters.values()).filter(
          c => c.severity === 'high'
        ).length,
        medium: Array.from(errorClusters.values()).filter(
          c => c.severity === 'medium'
        ).length,
        low: Array.from(errorClusters.values()).filter(
          c => c.severity === 'low'
        ).length,
      },
      recentErrors: errorHistory.filter(
        e => e.timestamp > new Date(Date.now() - 60 * 60 * 1000)
      ).length,
    };

    console.log('📊 Hourly Health Report:', report);

    return report;
  });

// Health check webhook
errorManagementWorkflow
  .onWebhook('/webhooks/health-check')
  .step('system-status', async ctx => {
    const systemHealth = await getSystemHealth();
    const activeClusters = errorClusters.size;
    const criticalClusters = Array.from(errorClusters.values()).filter(
      c => c.severity === 'critical'
    ).length;

    return {
      status: criticalClusters > 0 ? 'degraded' : systemHealth.status,
      systemHealth,
      errorClusters: {
        active: activeClusters,
        critical: criticalClusters,
      },
      timestamp: new Date().toISOString(),
    };
  });

console.log('🚨 Intelligent Error Management System Starting...');
console.log('🎯 Smart error clustering and automated response');
console.log('');
console.log('📡 Available endpoints:');
console.log('   POST /webhooks/error-event - Report new errors');
console.log('   GET  /webhooks/health-check - System health status');
console.log('');
console.log('🧠 AI Capabilities:');
console.log('   ✅ Real-time error clustering');
console.log('   ✅ Impact analysis (user tiers, system health)');
console.log('   ✅ Historical pattern matching');
console.log('   ✅ Automated diagnosis generation');
console.log('   ✅ Smart escalation rules');
console.log('   ✅ Automated remediation actions');
console.log('   ✅ Context-aware alerting');

export { errorManagementWorkflow, cleanupWorkflow, errorClusters };
```
