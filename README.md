<div align="center">

# ⚡ zdb

### The calm, fast database client that lives in your terminal.

PostgreSQL, MySQL & SQLite. Keyboard-first. Safe by default. Written in Rust.

![Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-supported-336791?logo=postgresql&logoColor=white)
![MySQL](https://img.shields.io/badge/MySQL-supported-4479A1?logo=mysql&logoColor=white)
![TUI](https://img.shields.io/badge/ratatui-powered-8A2BE2)

<br><br>

<img src="docs/screenshots/main.png" alt="zdb showing a SQL editor above a results grid" width="900">

</div>

---

## Why zdb?

Sometimes you just want to look at a table without launching a heavyweight GUI. You want something that starts quickly, stays out of your way, and **won't let you wreck production**.

`zdb` gives you a full SQL workbench inside your terminal, with an editor, a results grid, schema browsing, inline editing and an EXPLAIN viewer. It's all driven from the keyboard, and it works over SSH.

```bash
zdb                     # reopen the last project
zdb shop                # open a project (last environment, or the first non-prod one)
zdb shop:prod           # open a project on a specific environment
zdb postgres://…        # scratch connection (offers to save it when you quit)
zdb sqlite:///absolute/path/shop.db
zdb sqlite://relative/path/shop.db
zdb sqlite://:memory:    # temporary in-memory database
```

## ✨ Highlights

| | |
|---|---|
| 🐘🐬 **Postgres, MySQL & SQLite** | One interface for servers and local files, with TLS for server connections. |
| 🛡️ **Production-safe** | `prod`, `prd` and `live` environments open **read-only**, enforced on the server session. Destructive statements ask first. |
| ⌨️ **Keyboard-first** | A command palette (`^K`), a which-key menu on `Space`, vim-style navigation, and a fully remappable keymap. |
| 🧮 **Powerful results grid** | Server-side sorting, filter chips, find, multi-select, and follow foreign keys with `gd`. |
| ✏️ **Edit cells in place** | Stage edits, review them, and commit in **one transaction**. zdb refuses to commit if a row changed underneath you. |
| 📋 **Copy anything** | Values, CSV, JSON, Markdown, `INSERT` statements, column names or `IN (…)` lists, each a keystroke away. |
| 🌳 **EXPLAIN tree view** | Query plans as a readable tree, with per-node cost and time. |
| 🔐 **Secrets done right** | Passwords never touch your config. Use the OS keychain, `env:VAR`, `~/.pgpass`, `~/.my.cnf` or a prompt. |
| 🚇 **SSH tunnels built in** | Uses your system `ssh`, so `~/.ssh/config`, `ProxyJump` and your agent just work. |
| 🗂️ **Tabs, history & saved queries** | Stored locally in SQLite and shared between zdb instances. |
| 🎨 **Themeable** | Customise colour roles with ANSI names, 0–255 or `#rrggbb`. |

## 📸 A quick tour

<table>
<tr>
<td width="50%"><img src="docs/screenshots/prod.png" alt="Production environment: red banner and read-only"><br><sub><b>Production looks like production.</b> Red banner, read-only session.</sub></td>
<td width="50%"><img src="docs/screenshots/sidebar.png" alt="Sidebar with tables"><br><sub><b>Browse your schema</b> from the sidebar (<code>^B</code>).</sub></td>
</tr>
<tr>
<td width="50%"><img src="docs/screenshots/palette.png" alt="Fuzzy table picker"><br><sub><b>Fuzzy-jump to any table</b> with <code>^P</code>, or open the palette with <code>^K</code>.</sub></td>
<td width="50%"><img src="docs/screenshots/whichkey.png" alt="Which-key menu"><br><sub><b>Can't remember a key?</b> Press <code>Space</code> and zdb shows you.</sub></td>
</tr>
</table>

## 🚀 Quick start

```bash
# Build from source (requires a recent Rust toolchain)
git clone https://github.com/camwebby/zdb.git
cd zdb
cargo install --path .
```

> **Platforms:** developed and tested on macOS. Linux is built and tested in CI; Windows is untested.

Add a project to `~/.config/zdb/connections.toml`:

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

Then run `zdb shop` (or `zdb shop:prod`). `connections.toml` contains no secrets, so it's safe to commit and share with your team.

SQLite URLs also work in `connections.toml`. Files must already exist; relative paths resolve from the directory where you launch zdb. SQLite connections need no password or SSH tunnel. Production environments use SQLite's `query_only` mode until writes are unlocked. Schema browsing, foreign keys, transactional cell edits and `EXPLAIN QUERY PLAN` are supported; SQLite has no `EXPLAIN ANALYZE`.

## 🖥️ Layout

Four bands, nothing more:

1. **Top line:** project · environment (turns red on production)
2. **SQL editor**
3. **Results grid**
4. **Status line:** focus, stats, hints and messages

Press `^B` for a sidebar of tables and saved queries. Press `⏎` on a row to open the inspector and see it in full.

## 🛡️ Safety, built in

- **Production opens read-only.** Environments named `prod*`, `prd` or `live`, or with `level = "prod"`, are read-only on the server session. `Space c w` unlocks writes until you quit or go idle for 30 minutes.
- **Guard rails on dangerous SQL.** `UPDATE`/`DELETE` without `WHERE`, `DROP` and `TRUNCATE` all ask for confirmation. On production you must **type the environment name**.
- **Transactional edits.** Staged cell edits are reviewed (`^S`), committed atomically, and aborted if the underlying row has changed.
- **TLS follows libpq semantics.** `sslmode=require` and `prefer` encrypt the connection but do **not** verify the server certificate or hostname. Use `sslmode=verify-full` (or `verify-ca`) in the connection URL if you need protection against man-in-the-middle attacks.

## ⌨️ Keys at a glance

<details open>
<summary><b>Global</b></summary>

| Key | Action |
|---|---|
| `^K` | Command palette: `@` connections · `#` tables · `>` commands · `/` saved · `!` history |
| `^P` | Go to table |
| `^E` | Switch environment |
| `^R` / `^⏎` | Run statement (or selection) |
| `M-r` | Run all |
| `^C` | Cancel the running query (never quits) |
| `^T` / `^W` | New / close tab |
| `M-1…9` | Jump to tab (Option+1…9) |
| `^N` | Next tab (`t`/`T` in results) |
| `^J` | Editor ⇄ results (`Esc` leaves the editor, `Tab` cycles panes) |
| `^S` | Save query / review staged edits |
| `^X ^E` | Edit in `$EDITOR` |
| `^Q` | Quit |

</details>

<details>
<summary><b>Results grid</b></summary>

| Key | Action |
|---|---|
| `hjkl` / arrows, `g` / `G` | Navigate |
| `s` / `S` | Sort (server-side when the result was capped) |
| `v` / `V` / `M-v`, `⇧`+arrows, `⇧Space`, `^A` | Select cells, rows, or everything |
| `y` | Copy menu: `yy` values · `yc` CSV · `yC` CSV without headers · `yj` JSON · `ym` Markdown · `yi` INSERT · `yn` names · `yw` IN list |
| `/` `n` `N` | Find |
| `L` | Load all rows |
| `f` / `F` | Filter chips |
| `e` / `u` | Edit cell / undo edit |
| `gd` | Follow foreign key |
| `[` `]` | Previous / next result |

</details>

> 💡 **Lost?** Press `Space` and wait 300 ms for the which-key menu (`r` run, `y` copy, `c` connection, `t` table, `v` view, `x` export, `h` history, `q` saved, `f` format). Press `?` for help on the current pane.

In the editor, `^A`/`^E` (what Cmd+←/→ sends on macOS) jump to line start and end. Use `^E` from the results, or `^K`, to switch environment.

## ⚙️ Configuration

Lives in `$XDG_CONFIG_HOME/zdb` (or `~/.config/zdb`; override with `ZDB_CONFIG_DIR`).

| File | Purpose |
|---|---|
| `connections.toml` | Projects and environments. Secrets come from the keychain (default), `env:VAR`, `pgpass`, `mycnf`, `prompt` or `none`. |
| `config.toml` | Row limit, Space menu delay, vim mode, density, ASCII mode, mouse, and more. |
| `keymap.toml` | Remap anything with `action = "key"` in `[global]` and `[pane]` tables. Run `^K > settings: open keymap` for a commented template. Changes apply when the editor closes. |
| `theme.toml` | Colour roles: ANSI names, 0–255 or `#rrggbb`. |

History, workspaces and saved queries are stored in `$XDG_DATA_HOME/zdb/zdb.db` (override with `ZDB_DATA_DIR`).

## 🧪 Development

```bash
cargo test                                   # unit + render tests

ZDB_TEST_PG=postgres://app:secret@localhost:55432/shop \
ZDB_TEST_MYSQL=mysql://app:secret@localhost:53306/shop \
cargo test it_                               # drives the whole app against live servers
```

Spin up the test databases:

```bash
docker run -d --name zdb-pg -e POSTGRES_PASSWORD=secret -e POSTGRES_USER=app -e POSTGRES_DB=shop -p 55432:5432 postgres:17-alpine
docker run -d --name zdb-my -e MYSQL_ROOT_PASSWORD=secret -e MYSQL_DATABASE=shop -e MYSQL_USER=app -e MYSQL_PASSWORD=secret -p 53306:3306 mysql:8.4
```

<details>
<summary><b>Source map</b></summary>

- `sql.rs`: tokenizer, statement splitting, classification
- `db/`: drivers, schema, SSH tunnels
- `grid.rs` / `copy.rs` / `editor.rs`: models
- `app/`: state, actions, jobs, overlays
- `ui/`: rendering
- `keys.rs`: commands, keymap, Space menu

</details>

## 📄 License

[MIT](LICENSE)

---

<div align="center">

**Open a terminal. Run `zdb`. Query with confidence.**

Built with 🦀 [ratatui](https://ratatui.rs), [crossterm](https://github.com/crossterm-rs/crossterm) and [tokio](https://tokio.rs).

</div>
