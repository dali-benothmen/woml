interface WomlStateEntry<T = WomlJsonValue> {
  readonly found: true;
  readonly value: T;
  readonly version: number;
  readonly updatedAt: string;
}

interface WomlMissingStateEntry {
  readonly found: false;
}

interface WomlStateMutationOptions {
  readonly name: string;
  readonly ifVersion?: number;
}

interface WomlStateService {
  readonly get: <T = WomlJsonValue>(key: string) => Promise<WomlStateEntry<T> | WomlMissingStateEntry>;
  readonly has: (key: string) => Promise<{ readonly present: boolean; readonly version?: number }>;
  readonly set: (key: string, value: WomlJsonValue, options: WomlStateMutationOptions) => Promise<{ readonly stored: true; readonly version: number; readonly updatedAt: string }>;
  readonly delete: (key: string, options: WomlStateMutationOptions) => Promise<{ readonly deleted: boolean }>;
  readonly increment: (key: string, amount: number, options: WomlStateMutationOptions) => Promise<{ readonly value: number; readonly version: number; readonly updatedAt: string }>;
  readonly setIfAbsent: (key: string, value: WomlJsonValue, options: Readonly<{ readonly name: string }>) => Promise<{ readonly stored: boolean; readonly value: WomlJsonValue; readonly version: number; readonly updatedAt: string }>;
}
