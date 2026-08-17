import { cycleA } from './invalid-cycle-a.ts';

export function cycleB(): boolean {
  return cycleA();
}
