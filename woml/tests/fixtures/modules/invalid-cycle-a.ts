import { cycleB } from './invalid-cycle-b.ts';

export function cycleA(): boolean {
  return cycleB();
}
