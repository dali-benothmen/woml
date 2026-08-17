import { percentageMultiplier } from './pricing-helper.ts';

export function discount(price: number, percentage: number) {
  return { finalPrice: price * percentageMultiplier(percentage) };
}
