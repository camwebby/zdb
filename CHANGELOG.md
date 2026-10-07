# Changelog

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
