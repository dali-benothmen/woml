# Predictive Business Crisis Management

Early warning system and automated crisis response

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Types
interface CrisisSignal {
  source: 'social' | 'support' | 'system' | 'team' | 'news' | 'competitor';
  severity: number; // 0-10
  message: string;
  data: Record<string, any>;
  timestamp: Date;
}

interface CrisisScore {
  overall: number; // 0-100
  trend: 'improving' | 'stable' | 'worsening';
  level: 'normal' | 'elevated' | 'high' | 'critical';
  signals: CrisisSignal[];
  prediction: {
    likelyToEscalate: boolean;
    timeframe: string;
    confidence: number;
  };
}

interface CrisisResponse {
  level: 'watch' | 'alert' | 'mobilize' | 'emergency';
  actions: string[];
  stakeholders: string[];
  timeline: string;
  escalated: boolean;
}

// Mock data functions
async function getSocialSentiment(): Promise<CrisisSignal[]> {
  const signals: CrisisSignal[] = [];

  // Mock social media monitoring
  const negativeMentions = Math.floor(Math.random() * 50);
  const sentimentScore = 0.3 + Math.random() * 0.4; // 0.3-0.7 (lower is more negative)

  if (negativeMentions > 20 || sentimentScore < 0.4) {
    signals.push({
      source: 'social',
      severity: negativeMentions > 30 ? 8 : 6,
      message: `${negativeMentions} negative mentions detected, sentiment score: ${sentimentScore.toFixed(2)}`,
      data: { negativeMentions, sentimentScore, trending: Math.random() > 0.7 },
      timestamp: new Date(),
    });
  }

  return signals;
}

async function getSupportMetrics(): Promise<CrisisSignal[]> {
  const signals: CrisisSignal[] = [];

  // Mock support ticket analysis
  const currentTickets = Math.floor(Math.random() * 100) + 20;
  const baselineTickets = 30;
  const increase = ((currentTickets - baselineTickets) / baselineTickets) * 100;

  if (increase > 50) {
    signals.push({
      source: 'support',
      severity: increase > 150 ? 9 : increase > 100 ? 7 : 5,
      message: `Support tickets increased by ${increase.toFixed(1)}% (${currentTickets} vs baseline ${baselineTickets})`,
      data: {
        currentTickets,
        baselineTickets,
        increase,
        categories: ['login', 'payments', 'performance'],
      },
      timestamp: new Date(),
    });
  }

  // Mock escalated tickets
  if (Math.random() > 0.8) {
    signals.push({
      source: 'support',
      severity: 7,
      message: 'Multiple tickets escalated to engineering in last hour',
      data: { escalatedTickets: 5, timeframe: '1 hour' },
      timestamp: new Date(),
    });
  }

  return signals;
}

async function getSystemHealth(): Promise<CrisisSignal[]> {
  const signals: CrisisSignal[] = [];

  // Mock system performance monitoring
  const errorRate = Math.random() * 8;
  const responseTime = 200 + Math.random() * 800;
  const uptime = 95 + Math.random() * 5;

  if (errorRate > 3) {
    signals.push({
      source: 'system',
      severity: errorRate > 6 ? 8 : 6,
      message: `Error rate elevated: ${errorRate.toFixed(2)}%`,
      data: { errorRate, responseTime, uptime },
      timestamp: new Date(),
    });
  }

  if (responseTime > 500) {
    signals.push({
      source: 'system',
      severity: responseTime > 800 ? 7 : 5,
      message: `Response time degraded: ${responseTime.toFixed(0)}ms`,
      data: { responseTime, threshold: 300 },
      timestamp: new Date(),
    });
  }

  return signals;
}

async function getTeamCommunications(): Promise<CrisisSignal[]> {
  const signals: CrisisSignal[] = [];

  // Mock Slack/Teams analysis
  const keywords = ['bug', 'issue', 'problem', 'down', 'failing', 'urgent'];
  const mentionCounts = keywords.reduce(
    (acc, keyword) => {
      acc[keyword] = Math.floor(Math.random() * 10);
      return acc;
    },
    {} as Record<string, number>
  );

  const totalMentions = Object.values(mentionCounts).reduce(
    (sum, count) => sum + count,
    0
  );

  if (totalMentions > 15) {
    signals.push({
      source: 'team',
      severity: totalMentions > 25 ? 8 : 6,
      message: `Increased crisis-related team communications: ${totalMentions} mentions of issue keywords`,
      data: { mentionCounts, totalMentions, timeframe: 'last 2 hours' },
      timestamp: new Date(),
    });
  }

  return signals;
}

async function getNewsAnalysis(): Promise<CrisisSignal[]> {
  const signals: CrisisSignal[] = [];

  // Mock news/PR monitoring
  if (Math.random() > 0.9) {
    signals.push({
      source: 'news',
      severity: 6,
      message: 'Negative press coverage detected in industry publications',
      data: {
        publications: ['TechCrunch', 'Industry Weekly'],
        sentiment: 'negative',
      },
      timestamp: new Date(),
    });
  }

  return signals;
}

async function getCompetitorActivity(): Promise<CrisisSignal[]> {
  const signals: CrisisSignal[] = [];

  // Mock competitor monitoring
  if (Math.random() > 0.85) {
    const activities = [
      'product launch',
      'price cut',
      'acquisition',
      'funding round',
    ];
    const activity = activities[Math.floor(Math.random() * activities.length)];

    signals.push({
      source: 'competitor',
      severity: activity === 'price cut' ? 7 : 5,
      message: `Competitor ${activity} detected - potential market impact`,
      data: { activity, competitor: 'CompetitorX', impact: 'market_share' },
      timestamp: new Date(),
    });
  }

  return signals;
}

function calculateCrisisScore(allSignals: CrisisSignal[]): CrisisScore {
  // Weighted scoring by signal source
  const weights = {
    social: 1.2,
    support: 1.5,
    system: 1.3,
    team: 1.0,
    news: 0.8,
    competitor: 0.9,
  };

  let totalScore = 0;
  let maxScore = 0;

  allSignals.forEach(signal => {
    const weightedSeverity = signal.severity * weights[signal.source];
    totalScore += weightedSeverity;
    maxScore += 10 * weights[signal.source];
  });

  const overall = maxScore > 0 ? (totalScore / maxScore) * 100 : 0;

  // Determine level
  let level: CrisisScore['level'] = 'normal';
  if (overall > 75) level = 'critical';
  else if (overall > 50) level = 'high';
  else if (overall > 25) level = 'elevated';

  // Simple trend analysis (mock)
  const trend: CrisisScore['trend'] =
    Math.random() > 0.6
      ? 'worsening'
      : Math.random() > 0.5
        ? 'stable'
        : 'improving';

  // Prediction logic
  const likelyToEscalate = overall > 40 && trend === 'worsening';
  const timeframe =
    overall > 60 ? '1-2 hours' : overall > 40 ? '4-6 hours' : '12+ hours';
  const confidence = Math.min(0.95, 0.5 + (overall / 100) * 0.4);

  return {
    overall,
    trend,
    level,
    signals: allSignals,
    prediction: {
      likelyToEscalate,
      timeframe,
      confidence,
    },
  };
}

function generateCrisisResponse(crisisScore: CrisisScore): CrisisResponse {
  let level: CrisisResponse['level'] = 'watch';
  const actions: string[] = [];
  const stakeholders: string[] = [];
  let timeline = '';
  let escalated = false;

  // Determine response level
  if (crisisScore.level === 'critical') {
    level = 'emergency';
    timeline = 'Immediate action required';
    escalated = true;
  } else if (crisisScore.level === 'high') {
    level = 'mobilize';
    timeline = 'Response within 30 minutes';
    escalated = true;
  } else if (crisisScore.level === 'elevated') {
    level = 'alert';
    timeline = 'Response within 2 hours';
  }

  // Generate actions based on signal sources
  const signalSources = new Set(crisisScore.signals.map(s => s.source));

  if (signalSources.has('social')) {
    actions.push('Activate social media response team');
    actions.push('Prepare public communication strategy');
    stakeholders.push('PR Team', 'Social Media Manager');
  }

  if (signalSources.has('support')) {
    actions.push('Scale up customer support capacity');
    actions.push('Prepare customer communication templates');
    stakeholders.push('Support Team Lead', 'Customer Success');
  }

  if (signalSources.has('system')) {
    actions.push('Engage engineering team for system investigation');
    actions.push('Prepare system status page updates');
    stakeholders.push('Engineering Lead', 'DevOps Team');
  }

  if (signalSources.has('team')) {
    actions.push('Coordinate internal communication channels');
    actions.push('Establish incident command center');
    stakeholders.push('Operations Manager');
  }

  if (signalSources.has('news') || signalSources.has('competitor')) {
    actions.push('Brief executive team on market situation');
    actions.push('Prepare competitive response strategy');
    stakeholders.push('CEO', 'Marketing Director');
  }

  // Critical level additions
  if (level === 'emergency') {
    actions.push('Activate executive crisis team');
    actions.push('Prepare all-hands communication');
    stakeholders.push('CEO', 'CTO', 'VP of Operations');
  }

  return {
    level,
    actions: [...new Set(actions)], // Remove duplicates
    stakeholders: [...new Set(stakeholders)],
    timeline,
    escalated,
  };
}

async function executeCrisisResponse(
  response: CrisisResponse
): Promise<{ executed: string[]; failed: string[] }> {
  const executed: string[] = [];
  const failed: string[] = [];

  console.log(
    `🚨 Executing ${response.level.toUpperCase()} crisis response...`
  );

  for (const action of response.actions) {
    try {
      console.log(`   → ${action}`);

      // Mock action execution
      await new Promise(resolve => setTimeout(resolve, 100));

      // 95% success rate for mock
      if (Math.random() > 0.05) {
        executed.push(action);
      } else {
        failed.push(action);
        console.log(`   ❌ Failed: ${action}`);
      }
    } catch (error) {
      failed.push(action);
      console.error(`   ❌ Error executing ${action}:`, error);
    }
  }

  return { executed, failed };
}

// Main Crisis Management Workflow
const crisisManagementWorkflow = cronflow.define({
  id: 'predictive-crisis-management',
  name: 'Predictive Business Crisis Management',
  description: 'Early warning system and automated crisis response',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        console.log('🎯 Crisis monitoring cycle completed');
      }
    },
    onFailure: (ctx, stepId) => {
      console.error(
        `❌ Crisis management failed at step ${stepId}:`,
        ctx.step_error
      );
    },
  },
});

// Continuous monitoring (every 10 minutes)
crisisManagementWorkflow
  .onSchedule('*/10 * * * *')
  .step('collect-all-signals', async ctx => {
    console.log('🔍 Collecting crisis signals from all sources...');

    // Gather signals from all sources in parallel
    const [
      socialSignals,
      supportSignals,
      systemSignals,
      teamSignals,
      newsSignals,
      competitorSignals,
    ] = await Promise.all([
      getSocialSentiment(),
      getSupportMetrics(),
      getSystemHealth(),
      getTeamCommunications(),
      getNewsAnalysis(),
      getCompetitorActivity(),
    ]);

    const allSignals = [
      ...socialSignals,
      ...supportSignals,
      ...systemSignals,
      ...teamSignals,
      ...newsSignals,
      ...competitorSignals,
    ];

    console.log(`📊 Collected ${allSignals.length} crisis signals`);

    return { allSignals, timestamp: new Date().toISOString() };
  })
  .step('analyze-crisis-score', async ctx => {
    const { allSignals } = ctx.last;

    const crisisScore = calculateCrisisScore(allSignals);

    console.log(
      `📈 Crisis Score: ${crisisScore.overall.toFixed(1)}/100 (${crisisScore.level.toUpperCase()})`
    );
    console.log(
      `📊 Trend: ${crisisScore.trend} | Escalation likely: ${crisisScore.prediction.likelyToEscalate ? 'YES' : 'NO'}`
    );

    if (crisisScore.signals.length > 0) {
      console.log('🚨 Active signals:');
      crisisScore.signals.forEach(signal => {
        console.log(
          `   - ${signal.source.toUpperCase()}: ${signal.message} (severity: ${signal.severity})`
        );
      });
    }

    return { crisisScore };
  })
  .if('crisis-detected', ctx => ctx.last.crisisScore.level !== 'normal')
  .step('generate-response-plan', async ctx => {
    const { crisisScore } = ctx.last;

    const response = generateCrisisResponse(crisisScore);

    console.log(`🎯 Crisis response level: ${response.level.toUpperCase()}`);
    console.log(`⏰ Timeline: ${response.timeline}`);
    console.log(`👥 Stakeholders: ${response.stakeholders.join(', ')}`);
    console.log(`📋 Actions planned: ${response.actions.length}`);

    return { crisisScore, response };
  })
  .if('auto-response-needed', ctx => {
    const { response } = ctx.last;
    return response.level === 'emergency' || response.level === 'mobilize';
  })
  .step('execute-automated-response', async ctx => {
    const { response } = ctx.last;

    const executionResults = await executeCrisisResponse(response);

    console.log(
      `✅ Executed ${executionResults.executed.length} actions successfully`
    );
    if (executionResults.failed.length > 0) {
      console.log(
        `❌ Failed to execute ${executionResults.failed.length} actions`
      );
    }

    return { response, executionResults };
  })
  .step('notify-stakeholders', async ctx => {
    const { response, crisisScore } = ctx.last;

    console.log('📧 Notifying crisis stakeholders...');

    const notification = {
      level: response.level,
      crisisScore: crisisScore.overall,
      trend: crisisScore.trend,
      signals: crisisScore.signals.map(s => ({
        source: s.source,
        message: s.message,
        severity: s.severity,
      })),
      actionsExecuted: ctx.last.executionResults?.executed || [],
      stakeholders: response.stakeholders,
      timestamp: new Date().toISOString(),
    };

    // Mock sending notifications (Slack, email, SMS, etc.)
    response.stakeholders.forEach(stakeholder => {
      console.log(`   📱 Notified: ${stakeholder}`);
    });

    return { notificationsSent: response.stakeholders.length, notification };
  })
  .else()
  .step('escalate-for-review', async ctx => {
    const { response, crisisScore } = ctx.last;

    console.log(
      `📋 Crisis detected but requires human review (${response.level} level)`
    );
    console.log('   → Sending alert to crisis management team');

    return { escalated: true, requiresReview: true };
  })
  .endIf()
  .else()
  .action('log-normal-status', ctx => {
    console.log('✅ All systems normal - no crisis signals detected');
  })
  .endIf();

// Manual crisis trigger webhook
crisisManagementWorkflow
  .onWebhook('/webhooks/crisis-trigger', {
    schema: z.object({
      source: z.enum(['manual', 'external', 'system']),
      severity: z.number().min(1).max(10),
      description: z.string(),
      requiredResponse: z
        .enum(['watch', 'alert', 'mobilize', 'emergency'])
        .optional(),
    }),
  })
  .step('process-manual-trigger', async ctx => {
    const { source, severity, description, requiredResponse } = ctx.payload;

    console.log(`🚨 Manual crisis trigger activated: ${description}`);

    const manualSignal: CrisisSignal = {
      source: 'team', // Treat manual triggers as team communications
      severity,
      message: `Manual trigger: ${description}`,
      data: { source, manualTrigger: true },
      timestamp: new Date(),
    };

    const crisisScore = calculateCrisisScore([manualSignal]);

    // Override level if specified
    if (requiredResponse) {
      const levelMap = {
        watch: 'normal',
        alert: 'elevated',
        mobilize: 'high',
        emergency: 'critical',
      };
      crisisScore.level = levelMap[requiredResponse] as any;
    }

    return { manualSignal, crisisScore };
  })
  .step('execute-manual-response', async ctx => {
    const { crisisScore } = ctx.last;

    const response = generateCrisisResponse(crisisScore);
    const executionResults = await executeCrisisResponse(response);

    console.log(
      `🎯 Manual crisis response executed: ${response.level.toUpperCase()}`
    );

    return { response, executionResults };
  });

// Crisis status webhook
crisisManagementWorkflow
  .onWebhook('/webhooks/crisis-status')
  .step('get-current-status', async ctx => {
    // Get latest signals for status check
    const [socialSignals, supportSignals, systemSignals] = await Promise.all([
      getSocialSentiment(),
      getSupportMetrics(),
      getSystemHealth(),
    ]);

    const allSignals = [...socialSignals, ...supportSignals, ...systemSignals];
    const crisisScore = calculateCrisisScore(allSignals);

    return {
      status: crisisScore.level,
      score: crisisScore.overall,
      trend: crisisScore.trend,
      activeSignals: crisisScore.signals.length,
      prediction: crisisScore.prediction,
      timestamp: new Date().toISOString(),
    };
  });

console.log('🚨 Predictive Business Crisis Management System Starting...');
console.log('🔍 Monitoring 6 data sources every 10 minutes');
console.log('');
console.log('📡 Available endpoints:');
console.log('   POST /webhooks/crisis-trigger - Manual crisis activation');
console.log('   GET  /webhooks/crisis-status - Current crisis status');
console.log('');
console.log('📊 Monitoring Sources:');
console.log('   📱 Social media sentiment');
console.log('   🎧 Customer support volume');
console.log('   ⚙️  System performance metrics');
console.log('   💬 Team communications (Slack/Teams)');
console.log('   📰 External news & PR');
console.log('   🏢 Competitor activity');
console.log('');
console.log('🎯 Crisis Levels:');
console.log('   🟢 NORMAL: Business as usual');
console.log('   🟡 ELEVATED: Increased monitoring');
console.log('   🟠 HIGH: Active response required');
console.log('   🔴 CRITICAL: Emergency protocols activated');

export { crisisManagementWorkflow };
```
