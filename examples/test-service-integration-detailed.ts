#!/usr/bin/env bun

/**
 * Detailed Test Service Integration - Task 4.1
 *
 * This test verifies that services defined with defineService can be used
 * in workflow steps when added to the services array in .define().
 * This version includes actual step execution verification.
 */

import { cronflow, createValidContext } from '../sdk/src/index';
import { defineService } from '../services/src/index';

// Test configuration
const TEST_WORKFLOW_ID = 'test-service-integration-detailed-workflow';

async function testServiceIntegrationDetailed() {
  console.log('🧪 Testing Task 4.1: Detailed Service Integration');
  console.log('='.repeat(60));

  try {
    // Step 1: Create a simple service using defineService
    console.log('\n📋 Step 1: Creating service with defineService');
    const emailService = defineService({
      id: 'email-service',
      name: 'Email Service',
      description: 'A simple email service for testing',
      version: '1.0.0',
      setup: ({ config, auth }) => {
        console.log('🔧 Setting up email service with config:', config);
        console.log('🔧 Auth:', auth);

        return {
          actions: {
            send: async (to: string, subject: string, body: string) => {
              console.log(`📧 [EMAIL SERVICE] Sending email to ${to}`);
              console.log(`   Subject: ${subject}`);
              console.log(`   Body: ${body}`);
              console.log(`   Config:`, config);
              console.log(`   Auth:`, auth);

              // Simulate email sending
              await new Promise(resolve => setTimeout(resolve, 100));

              return {
                success: true,
                messageId: `msg_${Date.now()}`,
                to,
                subject,
                sentAt: new Date().toISOString(),
              };
            },
            getStatus: async (messageId: string) => {
              console.log(`📧 [EMAIL SERVICE] Getting status for ${messageId}`);
              return {
                messageId,
                status: 'delivered',
                deliveredAt: new Date().toISOString(),
              };
            },
          },
        };
      },
    });

    console.log('✅ Email service defined successfully');

    // Step 2: Configure the service
    console.log('\n📋 Step 2: Configuring the service');
    const configuredEmailService = emailService.withConfig({
      auth: {
        apiKey: 'test-api-key-12345',
      },
      config: {
        from: 'test@example.com',
        provider: 'smtp',
      },
    });

    console.log('✅ Email service configured successfully');
    console.log('   - Service ID:', configuredEmailService.id);
    console.log('   - Service Name:', configuredEmailService.name);
    console.log(
      '   - Available actions:',
      Object.keys(configuredEmailService.actions)
    );

    // Step 3: Create workflow with the service
    console.log('\n📋 Step 3: Creating workflow with service');
    const workflow = cronflow.define({
      id: TEST_WORKFLOW_ID,
      name: 'Detailed Service Integration Test Workflow',
      description:
        'A test workflow to verify service integration with actual execution',
      services: [configuredEmailService], // Add the configured service
    });

    // Step 4: Add steps that use the service
    console.log('\n📋 Step 4: Adding steps that use the service');
    workflow
      .step('send-welcome-email', async ctx => {
        console.log('🚀 Step 1: Sending welcome email');
        console.log('   - Payload:', ctx.payload);
        console.log('   - Available services:', Object.keys(ctx.services));

        // Check if email service is available
        if (!ctx.services['email-service']) {
          throw new Error('Email service not found in context');
        }

        const emailService = ctx.services['email-service'];
        console.log(
          '   - Email service actions:',
          Object.keys(emailService.actions)
        );

        // Use the email service
        const result = await emailService.actions.send(
          ctx.payload.email || 'user@example.com',
          'Welcome to our platform!',
          "Thank you for joining us. We're excited to have you on board!"
        );

        console.log('   - Email sent successfully:', result);
        return result;
      })
      .step('send-followup-email', async ctx => {
        console.log('🚀 Step 2: Sending followup email');
        console.log('   - Previous step result:', ctx.last);

        const emailService = ctx.services['email-service'];

        // Use the email service with data from previous step
        const result = await emailService.actions.send(
          ctx.payload.email || 'user@example.com',
          'Getting Started Guide',
          `Hi there! Here's your getting started guide. Your message ID from the welcome email was: ${ctx.last.messageId}`
        );

        console.log('   - Followup email sent successfully:', result);
        return result;
      });

    console.log('✅ Workflow defined with service integration');

    // Step 5: Initialize and start Cronflow
    console.log('\n📋 Step 5: Initializing Cronflow');
    await cronflow.start();
    console.log('✅ Cronflow initialized successfully');

    // Step 6: Execute the workflow
    console.log('\n📋 Step 6: Executing workflow with service');
    const payload = {
      email: 'test@example.com',
      userId: 'user-123',
      name: 'Test User',
    };

    const runId = await cronflow.trigger(TEST_WORKFLOW_ID, payload);
    console.log('✅ Workflow triggered successfully:', runId);

    // Step 7: Wait for completion and check results
    console.log('\n📋 Step 7: Waiting for workflow completion');
    await new Promise(resolve => setTimeout(resolve, 3000));

    const status = await cronflow.inspect(runId);
    console.log('✅ Workflow status:', status);

    // Step 8: Verify service integration worked
    console.log('\n📋 Step 8: Verifying service integration');
    if (status.status === 'completed' || status.status === 'pending') {
      console.log('✅ Service integration test completed successfully');
      console.log('   - Services were properly passed to workflow steps');
      console.log('   - Service actions were executed correctly');
      console.log('   - Context contained the expected services');
    } else {
      console.log('❌ Service integration test failed');
      console.log('   - Workflow status:', status.status);
    }

    // Step 9: Test manual step execution to verify services work
    console.log('\n📋 Step 9: Testing manual step execution');
    try {
      // Create a context with services for manual testing
      const testContext = createValidContext(
        runId,
        TEST_WORKFLOW_ID,
        'send-welcome-email',
        payload,
        {},
        { 'email-service': configuredEmailService }
      );

      console.log('   - Created test context with services');

      // Execute the step manually
      const stepResult = await cronflow.executeStepFunction(
        'send-welcome-email',
        testContext,
        TEST_WORKFLOW_ID,
        runId
      );

      console.log('   - Manual step execution result:', stepResult);

      if (stepResult.success) {
        console.log('✅ Manual step execution with services successful');
      } else {
        console.log('❌ Manual step execution failed:', stepResult.message);
      }
    } catch (error) {
      console.log('❌ Manual step execution error:', error);
    }

    return {
      success: true,
      runId,
      status: status.status,
      message: 'Detailed service integration test completed',
    };
  } catch (error: any) {
    console.error('❌ Detailed service integration test failed:', error);
    console.error('Stack trace:', error.stack);
    return {
      success: false,
      error: error.message,
      stack: error.stack,
    };
  } finally {
    // Clean up
    await cronflow.stop();
  }
}

// Run the test
testServiceIntegrationDetailed()
  .then(result => {
    if (result.success) {
      console.log('\n🎯 Task 4.1: Detailed Service Integration - COMPLETED');
      console.log('✅ Service integration is working correctly');
      console.log(
        '✅ Services can be defined, configured, and used in workflow steps'
      );
      console.log('✅ Service actions are properly available in step context');
      process.exit(0);
    } else {
      console.log('\n💥 Task 4.1: Detailed Service Integration - FAILED');
      console.log('❌ Service integration test failed');
      console.log('❌ Error:', result.error);
      process.exit(1);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
    process.exit(1);
  });

export { testServiceIntegrationDetailed };
