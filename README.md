# zdb

A calm, fast terminal UI for PostgreSQL and MySQL, built on ratatui and crossterm.

```
zdb                     reopen the last project
zdb shop                open a project (its last environment, or the first non-prod one)
zdb shop:prod           open a project on a specific environment
zdb postgres://…        scratch connection (offers to save it when you quit)
```

## Layout

Four bands: a top line (project · environment, red on production), the SQL editor, the
results, and a status line (focus, stats, hints, messages). A sidebar (`^B`) lists tables
and saved queries; the inspector (`⏎` on a row) shows one row in full.

## Keys

Global: `^K` palette (`@` connections, `#` tables, `>` commands, `/` saved, `!` history) ·
`^P` go to table · `^E` switch environment · `^R`/`^⏎` run statement (or selection) ·
`M-r` run all · `^C` cancel the running query (never quits) · `^T`/`^W` new/close tab ·
`M-1…9` (Option+1…9) jump to tab · `^N` next tab (`t`/`T` in results) · `^J` editor ⇄ results (`Esc` leaves the editor, `Tab` cycles panes) · `^S` save query / review staged edits · `^Q` quit · `^X ^E` edit in `$EDITOR`.

Results: `hjkl`/arrows · `g`/`G` · `s`/`S` sort (server-side when the result was capped) ·
`v`/`V`/`M-v` select · `⇧`+arrows extend · `⇧Space` select row · `^A` select all · `y` copy menu (`yy` values, `yc` CSV, `yC` CSV without headers, `yj`
JSON, `ym` Markdown, `yi` INSERT, `yn` names, `yw` IN list) · `/` `n` `N` find · `L` load all ·
`f`/`F` filter chips · `e` edit cell · `u` undo edit · `gd` follow foreign key · `[` `]` results.

`Space` opens a which-key menu after 300 ms (`r` run, `y` copy, `c` connection, `t` table,
`v` view, `x` export, `h` history, `q` saved, `f` format). `?` shows help for the current pane.

In the editor `^A`/`^E` (what Cmd+←/→ sends on macOS) are line start/end; switch environment
with `^E` from the results or `^K`.

## Safety

- Production environments (named `prod*`, `prd` or `live`, or with `level = "prod"`) open read-only; the
  session itself is read-only on the server. `Space c w` allows writes until you quit or
  stay idle for 30 minutes.
- UPDATE/DELETE without WHERE, DROP and TRUNCATE ask for confirmation; on production you type
  the environment name.
- Staged cell edits run in one transaction after a review screen (`^S`), and refuse to commit
  if a row changed underneath.

## Configuration

`$XDG_CONFIG_HOME/zdb` (or `~/.config/zdb`, override with `ZDB_CONFIG_DIR`):

- `connections.toml` — projects and environments. Safe to commit: passwords come from the OS
  keychain (default), `env:VAR`, `pgpass`, `mycnf`, `prompt` or `none`.
- `config.toml` — row limit, Space menu delay, vim mode, density, ascii, mouse, …
- `keymap.toml` — `[global]` and `[pane]` tables of `action = "key"`. `^K > settings: open
  keymap` writes a commented template listing every action, then opens it in `$EDITOR`;
  changes apply when the editor closes.
- `theme.toml` — colour roles (ANSI names, 0–255 or `#rrggbb`).

History, workspaces and saved queries live in `$XDG_DATA_HOME/zdb/zdb.db` (`ZDB_DATA_DIR`).

```toml
[[project]]
name = "shop"

[[project.env]]
name = "local"
url = "postgres://app@localhost:5432/shop"

[[project.env]]
name = "prod"
url = "postgres://app@db.internal/shop?sslmode=require"
ssh = "me@bastion.example.com"
```

## Development

```
cargo test                                  # unit + render tests
ZDB_TEST_PG=postgres://app:secret@localhost:55432/shop \
ZDB_TEST_MYSQL=mysql://app:secret@localhost:53306/shop \
cargo test it_                              # drives the whole app against live servers
```

Test servers:

```
docker run -d --name zdb-pg -e POSTGRES_PASSWORD=secret -e POSTGRES_USER=app -e POSTGRES_DB=shop -p 55432:5432 postgres:17-alpine
docker run -d --name zdb-my -e MYSQL_ROOT_PASSWORD=secret -e MYSQL_DATABASE=shop -e MYSQL_USER=app -e MYSQL_PASSWORD=secret -p 53306:3306 mysql:8.4
```

Source map: `sql.rs` (tokenizer, splitting, classification) · `db/` (drivers, schema) ·
`grid.rs`/`copy.rs`/`editor.rs` (models) · `app/` (state, actions, jobs, overlays) ·
`ui/` (rendering) · `keys.rs` (commands, keymap, Space menu).
