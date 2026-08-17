let invocationCalls = 0;

export function read(rows: unknown[][]) {
  invocationCalls += 1;
  return { rows, invocationCalls };
}

export async function removeEmptyRows(rows: unknown[][]) {
  invocationCalls += 1;
  return {
    rows: rows.filter(row => row.some(value => value !== '' && value != null)),
    invocationCalls,
  };
}
