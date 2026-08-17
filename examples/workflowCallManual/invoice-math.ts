export function calculateInvoice(
  price: number,
  quantity: number,
  taxRate: number
) {
  const subtotal = price * quantity;
  const tax = subtotal * taxRate;

  return {
    subtotal,
    tax,
    total: subtotal + tax,
  };
}
