#!/usr/bin/env bun

import { cronflow, type WebhookServerConfig } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';

const orderWorkflow = cronflow.define({
  id: 'external-order-processor',
  name: 'External Order Processor',
  description: 'Processes orders from external applications',
  tags: ['ecommerce', 'external'],
});

orderWorkflow
  .onWebhook('/api/orders/create')
  .step('validate-order', async (ctx: Context) => {
    console.log('📥 Received external order:', ctx.payload);

    const { orderId, amount, customerEmail } = ctx.payload;
    if (!orderId || !amount || !customerEmail) {
      throw new Error(
        'Missing required fields: orderId, amount, customerEmail'
      );
    }

    return {
      orderId,
      amount,
      customerEmail,
      processedAt: new Date().toISOString(),
    };
  })
  .step('process-payment', async (ctx: Context) => {
    const order = ctx.last;
    console.log(
      `💳 Processing payment for order ${order.orderId}: $${order.amount}`
    );

    await new Promise(resolve => setTimeout(resolve, 1000));

    return {
      ...order,
      paymentStatus: 'completed',
      transactionId: `txn_${Date.now()}`,
    };
  })
  .step('send-confirmation', async (ctx: Context) => {
    const order = ctx.last;
    console.log(`📧 Sending confirmation to ${order.customerEmail}`);

    await new Promise(resolve => setTimeout(resolve, 500));

    return {
      ...order,
      emailSent: true,
      emailId: `email_${Date.now()}`,
    };
  })
  .action('log-completion', (ctx: Context) => {
    const result = ctx.last;
    console.log(`✅ Order ${result.orderId} processed successfully!`);
    console.log(`   Payment: ${result.paymentStatus}`);
    console.log(`   Email: ${result.emailSent ? 'Sent' : 'Failed'}`);
  });

const userRegistrationWorkflow = cronflow.define({
  id: 'mobile-user-registration',
  name: 'Mobile User Registration',
  description: 'Handles user registration from mobile apps',
  tags: ['mobile', 'auth'],
});

userRegistrationWorkflow
  .onWebhook('/api/mobile/register')
  .step('validate-user-data', async (ctx: Context) => {
    const { email, password, deviceId } = ctx.payload;

    if (!email || !password) {
      throw new Error('Email and password are required');
    }

    return { email, password, deviceId, validatedAt: new Date().toISOString() };
  })
  .step('create-user-account', async (ctx: Context) => {
    const userData = ctx.last;
    console.log(`👤 Creating account for ${userData.email}`);

    await new Promise(resolve => setTimeout(resolve, 800));

    return {
      ...userData,
      userId: `user_${Date.now()}`,
      accountCreated: true,
    };
  })
  .step('send-welcome-email', async (ctx: Context) => {
    const user = ctx.last;
    console.log(`📧 Sending welcome email to ${user.email}`);

    await new Promise(resolve => setTimeout(resolve, 600));

    return {
      ...user,
      welcomeEmailSent: true,
    };
  })
  .action('log-registration', (ctx: Context) => {
    const user = ctx.last;
    console.log(`🎉 User ${user.email} registered successfully!`);
    console.log(`   User ID: ${user.userId}`);
    console.log(`   Device ID: ${user.deviceId}`);
  });

const iotDataWorkflow = cronflow.define({
  id: 'iot-data-processor',
  name: 'IoT Data Processor',
  description: 'Processes data from IoT devices',
  tags: ['iot', 'data'],
});

iotDataWorkflow
  .onWebhook('/api/iot/data')
  .step('parse-sensor-data', async (ctx: Context) => {
    const { deviceId, temperature, humidity, timestamp } = ctx.payload;

    console.log(`🌡️  Received data from device ${deviceId}:`);
    console.log(`   Temperature: ${temperature}°C`);
    console.log(`   Humidity: ${humidity}%`);

    return {
      deviceId,
      temperature: parseFloat(temperature),
      humidity: parseFloat(humidity),
      timestamp: new Date(timestamp),
      processedAt: new Date().toISOString(),
    };
  })
  .step('analyze-data', async (ctx: Context) => {
    const data = ctx.last;

    const isHighTemp = data.temperature > 30;
    const isHighHumidity = data.humidity > 80;

    return {
      ...data,
      alerts: {
        highTemperature: isHighTemp,
        highHumidity: isHighHumidity,
      },
      riskLevel: isHighTemp || isHighHumidity ? 'high' : 'normal',
    };
  })
  .if('needs-alert', (ctx: Context) => ctx.last.riskLevel === 'high')
  .step('send-alert', async (ctx: Context) => {
    const data = ctx.last;
    console.log(`🚨 Sending alert for device ${data.deviceId}`);
    console.log(`   Risk Level: ${data.riskLevel}`);

    await new Promise(resolve => setTimeout(resolve, 300));

    return {
      ...data,
      alertSent: true,
      alertId: `alert_${Date.now()}`,
    };
  })
  .endIf()
  .action('log-processing', (ctx: Context) => {
    const result = ctx.last;
    console.log(`📊 IoT data processed for device ${result.deviceId}`);
    console.log(`   Alerts: ${result.alertSent ? 'Sent' : 'None'}`);
  });

async function main() {
  console.log('🚀 Starting Cronflow with external webhook access...\n');

  const webhookConfig: WebhookServerConfig = {
    host: '0.0.0.0',
    port: 3000,
    maxConnections: 1000,
  };

  await cronflow.start({
    webhookServer: webhookConfig,
  });

  console.log('✅ Cronflow engine started successfully!');
  console.log('🌐 External webhook endpoints available:');
  console.log(
    '   📦 Orders: http://your-server:3000/webhook/api/orders/create'
  );
  console.log(
    '   📱 Mobile: http://your-server:3000/webhook/api/mobile/register'
  );
  console.log('   🔧 IoT:     http://your-server:3000/webhook/api/iot/data');
  console.log('   ❤️  Health:  http://your-server:3000/health');
  console.log('\n📋 Test with curl:');
  console.log(
    'curl -X POST http://your-server:3000/webhook/api/orders/create \\'
  );
  console.log('  -H "Content-Type: application/json" \\');
  console.log(
    '  -d \'{"orderId": "ord_123", "amount": 99.99, "customerEmail": "test@example.com"}\''
  );
  console.log('\n📱 Mobile apps can trigger workflows via HTTP requests');
  console.log('🌐 Web apps can integrate directly with these endpoints');
  console.log('🔧 External services can automate workflow execution');
}

main().catch(console.error);
