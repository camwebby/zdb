# Security Policy

zdb handles database credentials and can run write queries, so security reports are taken seriously.

## Reporting a vulnerability

Please **do not** open a public issue. Report privately using GitHub's
[private vulnerability reporting](https://github.com/camwebby/zdb/security/advisories/new),
or email hello@cameronwebby.com.

Include a description, reproduction steps and the affected version. You can expect an
acknowledgement within a few days.

## Scope notes

- `sslmode=require`/`prefer` encrypt but do not verify certificates (libpq semantics). This is
  documented behaviour; use `verify-full` for authenticated TLS.
- Passwords are stored in the OS keychain or read from `env:VAR`, `~/.pgpass` or `~/.my.cnf`.
  They should never be written to `connections.toml`.

## Supported versions

Only the latest release receives fixes.
