export function normalizeOrder(input: {
  orderId?: unknown;
  customerId?: unknown;
  amount?: unknown;
}) {
  const orderId = String(input.orderId ?? 'scheduled-order');
  const customerId = String(input.customerId ?? 'scheduled-customer');
  const amount = Number(input.amount ?? 0);
  if (!Number.isFinite(amount) || amount < 0) {
    throw new Error('Order amount must be a positive number.');
  }
  return { orderId, customerId, amount };
}

export function riskScore(amount: number, priorOrders: number) {
  return Math.min(100, Math.round(amount / 100 + priorOrders * 2));
}
