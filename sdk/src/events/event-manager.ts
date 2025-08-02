export interface EventListener {
  workflowId: string;
  trigger: any;
}

export interface EventHistoryItem {
  name: string;
  payload: any;
  timestamp: number;
}

let eventListeners: Map<string, Array<EventListener>> = new Map();
let eventHistory: Array<EventHistoryItem> = [];

export function setEventSystemState(
  listeners: Map<string, Array<EventListener>>,
  history: Array<EventHistoryItem>
): void {
  eventListeners = listeners;
  eventHistory = history;
}

export function getEventListenersState(): {
  listeners: Map<string, Array<EventListener>>;
  history: Array<EventHistoryItem>;
} {
  return { listeners: eventListeners, history: eventHistory };
}

export async function publishEvent(name: string, payload: any): Promise<void> {
  eventHistory.push({
    name,
    payload,
    timestamp: Date.now(),
  });

  if (eventHistory.length > 1000) {
    eventHistory = eventHistory.slice(-1000);
  }

  const listeners = eventListeners.get(name) || [];

  if (listeners.length === 0) {
    return;
  }

  const triggerPromises = listeners.map(async listener => {
    try {
      const { workflowId, trigger } = listener;

      const eventPayload = {
        event: {
          name,
          payload,
          timestamp: Date.now(),
        },
        ...payload,
      };

      const runId = await trigger(workflowId, eventPayload);

      return { workflowId, runId, success: true };
    } catch (error) {
      return { workflowId: listener.workflowId, error, success: false };
    }
  });

  const results = await Promise.allSettled(triggerPromises);

  const successful = results.filter(
    r => r.status === 'fulfilled' && r.value.success
  ).length;
  const failed = results.length - successful;

  results.forEach((result, index) => {
    if (result.status === 'rejected') {
    } else if (result.status === 'fulfilled' && !result.value.success) {
    }
  });
}

export function registerEventListener(
  eventName: string,
  workflowId: string,
  trigger: any
): void {
  if (!eventListeners.has(eventName)) {
    eventListeners.set(eventName, []);
  }

  const listeners = eventListeners.get(eventName)!;

  const existingIndex = listeners.findIndex(l => l.workflowId === workflowId);
  if (existingIndex >= 0) {
    listeners[existingIndex] = { workflowId, trigger };
  } else {
    listeners.push({ workflowId, trigger });
  }
}

export function unregisterEventListener(
  eventName: string,
  workflowId: string
): void {
  const listeners = eventListeners.get(eventName);
  if (!listeners) {
    return;
  }

  const index = listeners.findIndex(l => l.workflowId === workflowId);
  if (index >= 0) {
    listeners.splice(index, 1);

    if (listeners.length === 0) {
      eventListeners.delete(eventName);
    }
  }
}

export function getEventHistory(eventName?: string): Array<EventHistoryItem> {
  if (eventName) {
    return eventHistory.filter(event => event.name === eventName);
  }

  return [...eventHistory];
}

export function getEventListeners(
  eventName?: string
): Map<string, Array<EventListener>> {
  if (eventName) {
    const listeners = eventListeners.get(eventName);
    const result = new Map();
    if (listeners) {
      result.set(eventName, listeners);
    }
    return result;
  }

  return new Map(eventListeners);
}
