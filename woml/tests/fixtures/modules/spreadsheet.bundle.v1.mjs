// @bun
// values.ts
function hasValue(value) {
  return value !== null && value !== "";
}

// spreadsheet.ts
function read(rows) {
  return rows;
}
async function removeEmptyRows(rows) {
  return rows.filter((row) => row.some(hasValue));
}
export {
  removeEmptyRows,
  read
};
