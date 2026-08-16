import type {
  BuiltInCommunicationProvider,
  CommunicationTriggerAdapter,
} from './types';

/**
 * Starts provider trigger adapters as one runtime component and rolls back a
 * partial start in reverse order. It never decodes provider payloads or admits
 * runs itself.
 */
export class CommunicationTriggerHost {
  readonly #adapters: readonly CommunicationTriggerAdapter[];
  readonly #started: CommunicationTriggerAdapter[] = [];
  #state: 'created' | 'starting' | 'started' | 'closed' = 'created';

  constructor(adapters: readonly CommunicationTriggerAdapter[]) {
    const providers = new Set<BuiltInCommunicationProvider>();
    for (const adapter of adapters) {
      if (providers.has(adapter.provider)) {
        throw new Error(
          `Communication trigger adapter "${adapter.provider}" is registered more than once.`
        );
      }
      providers.add(adapter.provider);
    }
    this.#adapters = [...adapters];
  }

  providers(): readonly BuiltInCommunicationProvider[] {
    return this.#adapters.map(adapter => adapter.provider);
  }

  async start(): Promise<void> {
    if (this.#state !== 'created') {
      throw new Error('Communication trigger host cannot be started twice.');
    }
    this.#state = 'starting';
    try {
      for (const adapter of this.#adapters) {
        // Register ownership before startup so a partially initialized adapter
        // is still closed if its own start method fails.
        this.#started.push(adapter);
        await adapter.start();
      }
      this.#state = 'started';
    } catch (error) {
      await this.#closeStarted();
      this.#state = 'closed';
      throw error;
    }
  }

  async close(): Promise<void> {
    if (this.#state === 'closed') return;
    this.#state = 'closed';
    await this.#closeStarted();
  }

  async #closeStarted(): Promise<void> {
    const failures: unknown[] = [];
    for (const adapter of this.#started.splice(0).reverse()) {
      try {
        await adapter.close();
      } catch (error) {
        failures.push(error);
      }
    }
    if (failures.length > 0) {
      throw new AggregateError(
        failures,
        'One or more communication trigger adapters failed to close.'
      );
    }
  }
}
