# AI-Powered Content Moderation Agent

High-speed AI content moderation with human-in-the-loop fallback

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Types for better type safety
interface TextAnalysis {
  isToxic: boolean;
  confidence: number;
  categories: string[];
  reasoning: string;
}

interface ImageAnalysis {
  isInappropriate: boolean;
  confidence: number;
  categories: string[];
  reasoning: string;
}

interface ContextualAnalysis {
  riskScore: number;
  contextFlags: string[];
}

interface ContentMetadata {
  userAge?: number;
  isFirstPost: boolean;
  platform: string;
  timestamp?: string;
}

// AI Analysis Functions (using OpenAI-like API)
async function analyzeTextToxicity(text: string): Promise<TextAnalysis> {
  // Simulate AI analysis - replace with actual AI service
  const toxicWords = ['spam', 'hate', 'abuse', 'scam'];
  const foundToxic = toxicWords.some(word => text.toLowerCase().includes(word));

  return {
    isToxic: foundToxic,
    confidence: foundToxic ? 0.85 : 0.92,
    categories: foundToxic ? ['spam', 'inappropriate'] : [],
    reasoning: foundToxic
      ? 'Contains potentially harmful keywords'
      : 'Content appears safe',
  };
}

async function analyzeImageContent(imageUrl: string): Promise<ImageAnalysis> {
  // Simulate image analysis - replace with actual vision AI
  return {
    isInappropriate: false,
    confidence: 0.88,
    categories: [],
    reasoning: 'No inappropriate content detected',
  };
}

async function getContextualAnalysis(
  content: string,
  metadata: ContentMetadata
): Promise<ContextualAnalysis> {
  // Advanced contextual analysis
  let riskScore = 0;
  const contextFlags: string[] = [];

  if (metadata.userAge && metadata.userAge < 18) {
    riskScore += 0.2;
    contextFlags.push('minor_user');
  }

  if (metadata.isFirstPost) {
    riskScore += 0.1;
    contextFlags.push('new_user');
  }

  if (content.length < 10) {
    riskScore += 0.15;
    contextFlags.push('very_short_content');
  }

  return { riskScore, contextFlags };
}

// Logging and notification functions
async function logModerationDecision(data: any) {
  console.log('📊 Moderation Decision Logged:', {
    timestamp: new Date().toISOString(),
    contentId: data.contentId,
    decision: data.decision,
    confidence: data.confidence,
    processingTime: data.processingTime,
  });
}

async function sendSlackAlert(message: string, data: any) {
  console.log('🚨 Slack Alert:', message, data);
  // Replace with actual Slack webhook
}

async function notifyContentCreator(
  userId: string,
  decision: string,
  reasoning: string
) {
  console.log(`📧 Notifying user ${userId}: ${decision} - ${reasoning}`);
  // Replace with actual notification service
}

// Define the Content Moderation Workflow
const contentModerationWorkflow = cronflow.define({
  id: 'ai-content-moderator',
  name: 'AI-Powered Content Moderation Agent',
  description:
    'High-speed AI content moderation with human-in-the-loop fallback',
  hooks: {
    onSuccess: ctx => {
      console.log('✅ Content moderation completed successfully');
      console.log('📈 Performance:', {
        totalSteps: ctx.steps?.length || 0,
        finalDecision: ctx.last?.finalDecision || 'unknown',
      });
    },
    onFailure: ctx => {
      console.log('❌ Content moderation failed:', ctx.error);
      // Alert admins about system failure
    },
  },
});

contentModerationWorkflow
  .onWebhook('/webhooks/moderate-content', {
    schema: z.object({
      contentId: z.string(),
      contentType: z.enum(['text', 'image', 'video', 'mixed']),
      content: z.string(),
      imageUrl: z.string().optional(),
      userId: z.string(),
      metadata: z
        .object({
          userAge: z.number().optional(),
          isFirstPost: z.boolean().default(false),
          platform: z.string().default('web'),
          timestamp: z.string().optional(),
        })
        .optional(),
    }),
  })

  // Step 1: Initial content preprocessing
  .step('preprocess-content', async ctx => {
    console.log('🔍 Preprocessing content...');
    const { contentId, content, contentType, metadata } = ctx.payload;

    const startTime = Date.now();

    // Clean and normalize content
    const normalizedContent = content.trim().toLowerCase();
    const wordCount = content.split(' ').length;

    return {
      contentId,
      normalizedContent,
      originalContent: content,
      contentType,
      wordCount,
      metadata: metadata || {},
      startTime,
      processed: true,
    };
  })

  // Step 2: Run parallel AI analysis
  .parallel([
    // Text toxicity analysis
    async ctx => {
      console.log('🤖 Running toxicity analysis...');
      const textAnalysis = await analyzeTextToxicity(ctx.last.originalContent);
      return { textAnalysis };
    },

    // Image analysis (if applicable)
    async ctx => {
      if (ctx.payload.imageUrl && ctx.last.contentType !== 'text') {
        console.log('👁️  Running image analysis...');
        const imageAnalysis = await analyzeImageContent(ctx.payload.imageUrl);
        return { imageAnalysis };
      }
      return { imageAnalysis: null };
    },

    // Contextual risk assessment
    async ctx => {
      console.log('🎯 Running contextual analysis...');
      const contextAnalysis = await getContextualAnalysis(
        ctx.last.originalContent,
        ctx.last.metadata
      );
      return { contextAnalysis };
    },
  ])

  // Step 3: Combine AI results and make initial decision
  .step('combine-ai-results', async ctx => {
    console.log('🧠 Combining AI analysis results...');

    const [textResult, imageResult, contextResult] = ctx.last;
    const { textAnalysis } = textResult;
    const { imageAnalysis } = imageResult;
    const { contextAnalysis } = contextResult;

    // Calculate overall risk score
    let overallRisk = 0;
    const reasons: string[] = [];

    if (textAnalysis.isToxic) {
      overallRisk += textAnalysis.confidence * 0.6;
      reasons.push(`Text: ${textAnalysis.reasoning}`);
    }

    if (imageAnalysis?.isInappropriate) {
      overallRisk += imageAnalysis.confidence * 0.4;
      reasons.push(`Image: ${imageAnalysis.reasoning}`);
    }

    overallRisk += contextAnalysis.riskScore;
    if (contextAnalysis.contextFlags.length > 0) {
      reasons.push(`Context: ${contextAnalysis.contextFlags.join(', ')}`);
    }

    const confidence = Math.min(
      (textAnalysis.confidence + (imageAnalysis?.confidence || 0.9)) / 2,
      0.99
    );

    return {
      overallRisk,
      confidence,
      reasons,
      textAnalysis,
      imageAnalysis,
      contextAnalysis,
      combinedAt: Date.now(),
    };
  })

  // Step 4: Decision logic with confidence thresholds
  .if('needs-human-review', ctx => {
    const { overallRisk, confidence } = ctx.last;

    // Send to human review if:
    // - High risk but low confidence
    // - Medium risk with edge case indicators
    // - Any risk above 0.7 with confidence below 0.9
    return (
      (overallRisk > 0.3 && confidence < 0.8) ||
      (overallRisk > 0.7 && confidence < 0.9)
    );
  })

  // Human review branch
  .step('prep-human-review', async ctx => {
    console.log('👤 Preparing for human review...');

    const reviewData = {
      contentId: ctx.payload.contentId,
      content: ctx.payload.content,
      riskScore: ctx.last.overallRisk,
      confidence: ctx.last.confidence,
      reasons: ctx.last.reasons,
      aiAnalysis: {
        text: ctx.last.textAnalysis,
        image: ctx.last.imageAnalysis,
        context: ctx.last.contextAnalysis,
      },
    };

    return { reviewData, preparedAt: Date.now() };
  })

  .humanInTheLoop({
    timeout: '2h',
    description: 'Content requires human moderation review',
    onPause: async (ctx, token) => {
      console.log('🛑 Human review required');
      console.log(`🔑 Review token: ${token}`);

      await sendSlackAlert('Content flagged for human review', {
        contentId: ctx.last.reviewData.contentId,
        riskScore: ctx.last.reviewData.riskScore,
        token,
        reviewUrl: `https://admin.yoursite.com/review/${token}`,
      });
    },
  })

  .step('process-human-decision', async ctx => {
    console.log('✅ Processing human review decision...');

    if (ctx.last.timedOut) {
      return {
        finalDecision: 'auto-approved',
        reason: 'Human review timed out - defaulting to approval',
        reviewedBy: 'system',
        confidence: 0.5,
      };
    }

    return {
      finalDecision: ctx.last.approved ? 'approved' : 'rejected',
      reason: ctx.last.reason || 'Human moderator decision',
      reviewedBy: ctx.last.reviewerId || 'human-moderator',
      confidence: 1.0,
    };
  })

  .else()

  // Automated decision branch
  .step('make-automated-decision', async ctx => {
    console.log('🤖 Making automated moderation decision...');

    const { overallRisk, confidence } = ctx.last;

    let decision, reason;

    if (overallRisk > 0.6) {
      decision = 'rejected';
      reason = 'High risk content detected by AI';
    } else if (overallRisk > 0.3) {
      decision = 'flagged';
      reason = 'Medium risk - flagged for monitoring';
    } else {
      decision = 'approved';
      reason = 'Content passed AI safety checks';
    }

    return {
      finalDecision: decision,
      reason,
      reviewedBy: 'ai-system',
      confidence,
    };
  })

  .endIf()

  // Step 5: Execute decision and notifications
  .step('execute-decision', async ctx => {
    console.log('📤 Executing moderation decision...');

    const decision = ctx.last.finalDecision;
    const contentId = ctx.payload.contentId;
    const userId = ctx.payload.userId;
    const processingTime =
      Date.now() - ctx.steps['preprocess-content'].output.startTime;

    // Log the decision
    await logModerationDecision({
      contentId,
      decision,
      confidence: ctx.last.confidence,
      processingTime,
      reviewedBy: ctx.last.reviewedBy,
    });

    // Notify content creator if needed
    if (decision === 'rejected' || decision === 'flagged') {
      await notifyContentCreator(userId, decision, ctx.last.reason);
    }

    return {
      contentId,
      decision,
      processingTime,
      notificationSent: decision !== 'approved',
      executedAt: Date.now(),
    };
  })

  // Background action: Update analytics
  .action('update-analytics', async ctx => {
    // This runs in background and doesn't block the workflow
    console.log('📊 Updating moderation analytics...');

    const decision = ctx.last.decision;
    const processingTime = ctx.last.processingTime;

    // Update your analytics/metrics system here
    console.log('Analytics updated:', {
      decision,
      processingTime,
      timestamp: new Date().toISOString(),
    });
  });

// Start the Cronflow engine
console.log('🚀 Starting AI Content Moderation Agent...');
cronflow.start();

console.log('📋 Available endpoints:');
console.log(
  '  POST /webhooks/moderate-content - Submit content for moderation'
);
console.log('');
console.log('📖 Example usage:');
console.log(`curl -X POST http://localhost:3000/webhooks/moderate-content \\
  -H "Content-Type: application/json" \\
  -d '{
    "contentId": "post_123",
    "contentType": "text",
    "content": "This is a sample post to moderate",
    "userId": "user_456",
    "metadata": {
      "userAge": 25,
      "isFirstPost": false,
      "platform": "web"
    }
  }'`);

console.log('');
console.log('🔧 To resume paused workflows (human reviews):');
console.log(
  '  cronflow.resume("approval_token", { approved: true, reason: "Looks good", reviewerId: "mod_123" })'
);

// Export for external use
export { cronflow, contentModerationWorkflow };
```
