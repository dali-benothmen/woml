export const SCHEDULE_CRON_DIALECT = 'woml-cron-v1' as const;

interface CronFieldDefinition {
  readonly name: string;
  readonly minimum: number;
  readonly maximum: number;
  readonly normalize?: (value: number) => number;
}

interface ParsedCronField {
  readonly values: ReadonlySet<number>;
  readonly unrestricted: boolean;
}

export interface ParsedScheduleCron {
  readonly dialect: typeof SCHEDULE_CRON_DIALECT;
  readonly source: string;
  readonly minute: ParsedCronField;
  readonly hour: ParsedCronField;
  readonly dayOfMonth: ParsedCronField;
  readonly month: ParsedCronField;
  readonly dayOfWeek: ParsedCronField;
}

export interface ScheduleWallTime {
  readonly year: number;
  readonly month: number;
  readonly day: number;
  readonly hour: number;
  readonly minute: number;
}

const fields: readonly CronFieldDefinition[] = [
  { name: 'minute', minimum: 0, maximum: 59 },
  { name: 'hour', minimum: 0, maximum: 23 },
  { name: 'day-of-month', minimum: 1, maximum: 31 },
  { name: 'month', minimum: 1, maximum: 12 },
  {
    name: 'day-of-week',
    minimum: 0,
    maximum: 7,
    normalize: value => (value === 7 ? 0 : value),
  },
];

const formatterCache = new Map<string, Intl.DateTimeFormat>();
const supportedTimeZones = new Set(Intl.supportedValuesOf('timeZone'));

export class ScheduleCronSyntaxError extends Error {
  constructor(readonly reason: string) {
    super(reason);
    this.name = 'ScheduleCronSyntaxError';
  }
}

function integer(value: string): number | undefined {
  if (!/^[0-9]+$/.test(value)) return undefined;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}

function inBounds(value: number, field: CronFieldDefinition): boolean {
  return value >= field.minimum && value <= field.maximum;
}

function fieldValues(
  source: string,
  field: CronFieldDefinition
): ParsedCronField {
  if (source.length === 0) {
    throw new ScheduleCronSyntaxError(`${field.name} must not be empty`);
  }
  const values = new Set<number>();
  for (const listItem of source.split(',')) {
    if (listItem.length === 0) {
      throw new ScheduleCronSyntaxError(
        `${field.name} contains an empty list item`
      );
    }
    const stepParts = listItem.split('/');
    if (stepParts.length > 2 || stepParts.some(part => part.length === 0)) {
      throw new ScheduleCronSyntaxError(
        `${field.name} contains an invalid step`
      );
    }
    const base = stepParts[0]!;
    const step = stepParts[1] === undefined ? 1 : integer(stepParts[1]);
    if (step === undefined || step < 1) {
      throw new ScheduleCronSyntaxError(
        `${field.name} step must be a positive integer`
      );
    }

    let start: number;
    let end: number;
    if (base === '*') {
      start = field.minimum;
      end = field.maximum;
    } else if (base.includes('-')) {
      const range = base.split('-');
      if (range.length !== 2) {
        throw new ScheduleCronSyntaxError(
          `${field.name} contains an invalid range`
        );
      }
      const parsedStart = integer(range[0]!);
      const parsedEnd = integer(range[1]!);
      if (
        parsedStart === undefined ||
        parsedEnd === undefined ||
        !inBounds(parsedStart, field) ||
        !inBounds(parsedEnd, field) ||
        parsedStart > parsedEnd
      ) {
        throw new ScheduleCronSyntaxError(
          `${field.name} range is outside ${field.minimum}-${field.maximum}`
        );
      }
      start = parsedStart;
      end = parsedEnd;
    } else {
      const parsed = integer(base);
      if (parsed === undefined || !inBounds(parsed, field)) {
        throw new ScheduleCronSyntaxError(
          `${field.name} value is outside ${field.minimum}-${field.maximum}`
        );
      }
      start = parsed;
      end = stepParts[1] === undefined ? parsed : field.maximum;
    }

    for (let value = start; value <= end; value += step) {
      values.add(field.normalize?.(value) ?? value);
    }
  }

  const everyValue = new Set<number>();
  for (let value = field.minimum; value <= field.maximum; value += 1) {
    everyValue.add(field.normalize?.(value) ?? value);
  }
  return {
    values,
    unrestricted:
      values.size === everyValue.size &&
      [...everyValue].every(value => values.has(value)),
  };
}

export function parseScheduleCron(source: string): ParsedScheduleCron {
  if (source.length > 256) {
    throw new ScheduleCronSyntaxError('cron must not exceed 256 characters');
  }
  if (source !== source.trim() || /\s/.test(source.replace(/ /g, ''))) {
    throw new ScheduleCronSyntaxError(
      'cron must use single ASCII spaces between fields'
    );
  }
  const parts = source.split(' ');
  if (parts.length !== 5 || parts.some(part => part.length === 0)) {
    throw new ScheduleCronSyntaxError(
      'cron must contain exactly five fields separated by single spaces'
    );
  }
  const parsed = parts.map((part, index) => fieldValues(part, fields[index]!));
  return {
    dialect: SCHEDULE_CRON_DIALECT,
    source,
    minute: parsed[0]!,
    hour: parsed[1]!,
    dayOfMonth: parsed[2]!,
    month: parsed[3]!,
    dayOfWeek: parsed[4]!,
  };
}

export function isSupportedScheduleTimeZone(value: string): boolean {
  return supportedTimeZones.has(value);
}

function dayOfWeek(wall: ScheduleWallTime): number {
  return new Date(Date.UTC(wall.year, wall.month - 1, wall.day)).getUTCDay();
}

export function scheduleCronMatches(
  cron: ParsedScheduleCron,
  wall: ScheduleWallTime
): boolean {
  if (
    !cron.minute.values.has(wall.minute) ||
    !cron.hour.values.has(wall.hour) ||
    !cron.month.values.has(wall.month)
  ) {
    return false;
  }
  const dayOfMonthMatches = cron.dayOfMonth.values.has(wall.day);
  const dayOfWeekMatches = cron.dayOfWeek.values.has(dayOfWeek(wall));
  if (cron.dayOfMonth.unrestricted) return dayOfWeekMatches;
  if (cron.dayOfWeek.unrestricted) return dayOfMonthMatches;
  return dayOfMonthMatches || dayOfWeekMatches;
}

function formatter(timeZone: string): Intl.DateTimeFormat {
  const existing = formatterCache.get(timeZone);
  if (existing !== undefined) return existing;
  const created = new Intl.DateTimeFormat('en-CA-u-ca-iso8601-nu-latn', {
    timeZone,
    calendar: 'iso8601',
    numberingSystem: 'latn',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  });
  formatterCache.set(timeZone, created);
  return created;
}

export function scheduleWallTime(
  instant: Date,
  timeZone: string
): ScheduleWallTime {
  if (!isSupportedScheduleTimeZone(timeZone)) {
    throw new RangeError(`Unsupported schedule timezone: ${timeZone}`);
  }
  const values = new Map(
    formatter(timeZone)
      .formatToParts(instant)
      .filter(part => part.type !== 'literal')
      .map(part => [part.type, Number(part.value)])
  );
  return {
    year: values.get('year')!,
    month: values.get('month')!,
    day: values.get('day')!,
    hour: values.get('hour')!,
    minute: values.get('minute')!,
  };
}

export function scheduleOccurrencesBetween(
  cronSource: string,
  timeZone: string,
  startInclusive: string,
  endExclusive: string
): readonly string[] {
  const cron = parseScheduleCron(cronSource);
  if (!isSupportedScheduleTimeZone(timeZone)) {
    throw new RangeError(`Unsupported schedule timezone: ${timeZone}`);
  }
  const start = Date.parse(startInclusive);
  const end = Date.parse(endExclusive);
  if (!Number.isFinite(start) || !Number.isFinite(end) || start >= end) {
    throw new RangeError('Schedule fixture window must be a valid increasing interval.');
  }
  const firstMinute = Math.ceil(start / 60_000) * 60_000;
  const minuteCount = Math.ceil((end - firstMinute) / 60_000);
  if (minuteCount > 600_000) {
    throw new RangeError('Schedule fixture windows may not exceed 600,000 minutes.');
  }
  const occurrences: string[] = [];
  for (let instant = firstMinute; instant < end; instant += 60_000) {
    const date = new Date(instant);
    if (scheduleCronMatches(cron, scheduleWallTime(date, timeZone))) {
      occurrences.push(date.toISOString());
    }
  }
  return occurrences;
}
