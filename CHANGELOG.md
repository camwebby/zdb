# Changelog

## Unreleased

- Add `a` to insert a row from a form in table tabs, and `A` to start it from a copy of the current row. The form lists every writable column with its type, key and required markers, shows what an empty field will do (the column default, `NULL`, or required), previews the `INSERT`, and reopens with your values if the database rejects it. `^N` cycles a field between default, `NULL` and an empty string. The new row appears at the top of the table.
- Add `D` to delete the row under the cursor, or every selected row, from a table tab. It asks for confirmation, runs in one transaction, and keeps your place in the results.

## 0.1.3

- Fix a crash when scrolling near the bottom of results and enlarging the terminal or result grid.

## 0.1.2

- Fix intermittent macOS clipboard errors when copying CSV with headers and other formats by avoiding competing native and terminal clipboard writes.
- Report terminal clipboard write failures instead of showing a successful copy.

## 0.1.1

- Prefill table filters from the selected cell and support plain-text searches across columns.
- Remove the most recent filter with Backspace and show clearer filter hints.
- Improve row selection with Shift+arrow keys and preserve the selected column after sorting.
- Show more useful result-grid shortcuts and add optional key diagnostics with `ZDB_DEBUG_KEYS`.

## 0.1.0

- Initial public release: terminal UI for PostgreSQL and MySQL with SQL editor, results grid,
  schema browser, inline editing, EXPLAIN viewer, SSH tunnels and production-safe defaults.
