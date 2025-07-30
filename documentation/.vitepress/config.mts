import { defineConfig } from 'vitepress';

export default defineConfig({
  title: 'Cronflow',
  description: 'The Fastest Code-First Workflow Automation Engine',
  lang: 'en-US',
  outDir: '../build',
  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }],
    ['meta', { name: 'theme-color', content: '#22c55e' }],
    [
      'meta',
      { name: 'viewport', content: 'width=device-width, initial-scale=1.0' },
    ],
  ],
  themeConfig: {
    logo: '/logo.png',
    siteTitle: 'Cronflow',
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Guide', link: '/guide/' },
      { text: 'API', link: '/api/' },
      { text: 'Examples', link: '/examples/' },
      { text: 'GitHub', link: 'https://github.com/dali-benothmen/cronflow' },
    ],
    sidebar: {
      '/guide/': [
        {
          text: 'Getting Started',
          items: [
            { text: 'Introduction', link: '/guide/' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quick Start', link: '/guide/quick-start' },
            { text: 'Basic Concepts', link: '/guide/basic-concepts' },
          ],
        },
        {
          text: 'Core Features',
          items: [
            { text: 'Workflows', link: '/guide/workflows' },
            { text: 'Steps', link: '/guide/steps' },
            { text: 'Actions', link: '/guide/actions' },
            { text: 'Triggers', link: '/guide/triggers' },
            { text: 'State Management', link: '/guide/state-management' },
            { text: 'Custom Steps', link: '/guide/custom-steps' },
            { text: 'Hooks', link: '/guide/hooks' },
            { text: 'Conditions', link: '/guide/conditions' },
            { text: 'Error Handling', link: '/guide/error-handling' },
            { text: 'Performance', link: '/guide/performance' },
          ],
        },
        {
          text: 'Advanced',
          items: [
            { text: 'Parallel Execution', link: '/guide/parallel-execution' },
            {
              text: 'Framework Integration',
              link: '/guide/framework-integration',
            },
            { text: 'Advanced Triggers', link: '/guide/advanced-triggers' },
            { text: 'State Management', link: '/guide/state-management' },
            { text: 'Testing', link: '/guide/testing' },
            { text: 'Background Actions', link: '/guide/background-actions' },
            { text: 'Human-in-the-Loop', link: '/guide/human-in-the-loop' },
            { text: 'Batch Processing', link: '/guide/batch-processing' },
            {
              text: 'Advanced Control Flow',
              link: '/guide/advanced-control-flow',
            },
          ],
        },
      ],
      '/api/': [
        {
          text: 'API Reference',
          items: [{ text: 'Overview', link: '/api/' }],
        },
        {
          text: 'Core Functions',
          items: [
            { text: 'Core Namespace', link: '/api/core-namespace' },
            { text: 'State Management', link: '/api/state-management' },
            { text: 'Workflow Execution', link: '/api/workflow-execution' },
            { text: 'Human-in-the-Loop', link: '/api/human-in-the-loop' },
            { text: 'Trigger Management', link: '/api/trigger-management' },
            { text: 'Event Management', link: '/api/event-management' },
            { text: 'Hook Management', link: '/api/hook-management' },
            { text: 'Context Management', link: '/api/context-management' },
            {
              text: 'Core Status & Performance',
              link: '/api/core-status-performance',
            },
          ],
        },
        {
          text: 'Workflow Instance Methods',
          items: [
            {
              text: 'Workflow Instance Methods',
              link: '/api/workflow-instance-methods',
            },
            { text: 'Trigger Methods', link: '/api/trigger-methods' },
            { text: 'Control Flow Methods', link: '/api/control-flow-methods' },
            {
              text: 'Advanced Control Flow',
              link: '/api/advanced-control-flow',
            },
            { text: 'Workflow Management', link: '/api/workflow-management' },
            { text: 'Testing API', link: '/api/testing-api' },
          ],
        },
        {
          text: 'Reference',
          items: [
            { text: 'Context Object', link: '/api/context-object' },
            { text: 'Configuration', link: '/api/configuration' },
          ],
        },
      ],
      '/examples/': [
        {
          text: 'Examples',
          items: [
            { text: 'Overview', link: '/examples/' },
            {
              text: 'AgriMind - Smart Farming AI',
              link: '/examples/agriMindWorkflow',
            },
            { text: 'AI Chief of Staff', link: '/examples/aiChiefOfStaff' },
            {
              text: 'AI Healthcare Triage Agent',
              link: '/examples/aiHealthcareTriageWorkflow',
            },
            {
              text: 'AI Trading & Market Intelligence',
              link: '/examples/aiTradingAgent',
            },
            {
              text: 'AI-Powered Content Moderation',
              link: '/examples/contentModerationWorkflow',
            },
            {
              text: 'AI Financial Fraud Detection',
              link: '/examples/fraudDetectionWorkflow',
            },
            {
              text: 'AI Hiring & Recruitment Agent',
              link: '/examples/hiringWorkflow',
            },
            {
              text: 'MicroFlip - Domain Flipping',
              link: '/examples/microFlipWorkflow',
            },
            {
              text: 'Intelligent Database Sync',
              link: '/examples/databaseSyncWorkflow',
            },
            {
              text: 'Intelligent Expense Management',
              link: '/examples/expenseWorkflow',
            },
            {
              text: 'Investment Portfolio Rebalancing',
              link: '/examples/portfolioRebalancingWorkflow',
            },
            {
              text: 'Educational Progress Tracking',
              link: '/examples/educationTrackingWorkflow',
            },
            {
              text: 'Intelligent Error Management',
              link: '/examples/errorManagementWorkflow',
            },
            {
              text: 'Crisis Management',
              link: '/examples/crisisManagementWorkflow',
            },
          ],
        },
      ],
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/dali-benothmen/cronflow' },
    ],
    footer: {
      message: 'Released under the Apache 2.0 License.',
      copyright: 'Copyright © 2024-present Mohamed Ali Ben Othmen',
    },
    search: {
      provider: 'local',
    },
  },
  markdown: {
    theme: 'material-theme-palenight',
    lineNumbers: true,
  },
});
