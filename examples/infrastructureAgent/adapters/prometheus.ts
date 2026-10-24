#!/usr/bin/env bun

const [command] = Bun.argv.slice(2);
if (command !== 'health') {
  console.error('Usage: prometheus.ts health');
  process.exit(2);
}
console.log(JSON.stringify({
  query: 'sum(rate(http_requests_total{service="payments"}[5m]))',
  successRate: 99.99,
  errorBudgetRemaining: 87.4,
  healthy: true,
}));
