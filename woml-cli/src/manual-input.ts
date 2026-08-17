import { createInterface, type Interface } from 'node:readline';

export interface ManualLineInput {
  readonly isTTY: boolean;
  run(onLine: (line: string) => void | Promise<void>): Promise<void>;
  close(): void;
}

class ProcessManualLineInput implements ManualLineInput {
  readonly isTTY = process.stdin.isTTY === true;
  readonly #readline: Interface;
  #closed = false;

  constructor() {
    this.#readline = createInterface({
      input: process.stdin,
      terminal: this.isTTY,
      crlfDelay: Infinity,
    });
  }

  async run(onLine: (line: string) => void | Promise<void>): Promise<void> {
    for await (const line of this.#readline) {
      if (this.#closed) break;
      await onLine(line);
    }
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#readline.close();
  }
}

export function createProcessManualLineInput(): ManualLineInput {
  return new ProcessManualLineInput();
}
