import { describe, expect, test } from 'bun:test';
import Ajv2020 from 'ajv/dist/2020';

import {
  isSupportedScheduleTimeZone,
  parseScheduleCron,
  scheduleCronMatches,
  scheduleOccurrencesBetween,
  ScheduleCronSyntaxError,
  SCHEDULE_CRON_DIALECT,
} from '../src/schedule';

interface ScheduleSemanticsFixture {
  readonly contract: string;
  readonly contractVersion: number;
  readonly dialect: string;
  readonly occurrenceCases: readonly {
    readonly name: string;
    readonly cron: string;
    readonly timezone: string;
    readonly startInclusive: string;
    readonly endExclusive: string;
    readonly expected: readonly string[];
  }[];
  readonly invalidCron: readonly string[];
  readonly invalidTimezones: readonly string[];
}

const fixture = (await Bun.file(
  new URL('./fixtures/schedule-semantics.v1.json', import.meta.url)
).json()) as ScheduleSemanticsFixture;

describe('T8 WOML Cron v1 semantics', () => {
  test('validates the versioned schedule semantics artifact', async () => {
    const schema = await Bun.file(
      new URL(
        '../../docs/schemas/schedule-semantics.v1.schema.json',
        import.meta.url
      )
    ).json();
    const ajv = new Ajv2020({
      allErrors: true,
      strict: false,
      validateFormats: false,
    });
    const validate = ajv.compile(schema);
    expect(validate(fixture), JSON.stringify(validate.errors)).toBe(true);
  });

  test('pins every ordinary, boundary, leap, offset, and DST occurrence', () => {
    expect(fixture).toMatchObject({
      contract: 'woml.schedule-semantics',
      contractVersion: 1,
      dialect: SCHEDULE_CRON_DIALECT,
    });
    for (const entry of fixture.occurrenceCases) {
      expect(
        scheduleOccurrencesBetween(
          entry.cron,
          entry.timezone,
          entry.startInclusive,
          entry.endExclusive
        ),
        entry.name
      ).toEqual(entry.expected);
    }
  });

  test('supports numeric lists, ranges, wildcard steps, range steps, and value steps', () => {
    const cron = parseScheduleCron('0,30 8-18/2 1,15 */3 1-5');
    expect(cron.minute.values).toEqual(new Set([0, 30]));
    expect(cron.hour.values).toEqual(new Set([8, 10, 12, 14, 16, 18]));
    expect(cron.month.values).toEqual(new Set([1, 4, 7, 10]));
    expect(
      scheduleCronMatches(cron, {
        year: 2026,
        month: 4,
        day: 1,
        hour: 10,
        minute: 30,
      })
    ).toBe(true);

    expect(parseScheduleCron('5/20 * * * *').minute.values).toEqual(
      new Set([5, 25, 45])
    );
  });

  test('rejects every expression outside the frozen numeric five-field dialect', () => {
    for (const source of fixture.invalidCron) {
      expect(() => parseScheduleCron(source), source).toThrow(
        ScheduleCronSyntaxError
      );
    }
  });

  test('accepts canonical IANA zones and rejects aliases and local/offset guesses', () => {
    for (const timezone of [
      'UTC',
      'Europe/Berlin',
      'America/New_York',
      'Asia/Kathmandu',
      'Etc/GMT+5',
    ]) {
      expect(isSupportedScheduleTimeZone(timezone), timezone).toBe(true);
    }
    for (const timezone of fixture.invalidTimezones) {
      expect(isSupportedScheduleTimeZone(timezone), timezone).toBe(false);
    }
  });
});
