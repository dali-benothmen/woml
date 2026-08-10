import { hasValue } from './values.ts';

export function read(rows: unknown[][]) {
  return rows;
}

export async function removeEmptyRows(rows: unknown[][]) {
  return rows.filter(row => row.some(hasValue));
}

export type SpreadsheetRow = unknown[];
