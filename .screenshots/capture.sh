#!/bin/sh
# Captures ANSI-coloured pane dumps of zdb into .screenshots/out/*.ansi
# Usage: run from repo root. Requires tmux, a built release binary and the seeded demo DB.
set -e
OUT=.screenshots/out
mkdir -p $OUT
ENVV="ZDB_CONFIG_DIR=$PWD/.screenshots/cfg ZDB_DATA_DIR=$PWD/.screenshots/data TERM=xterm-256color COLORTERM=truecolor"
start() { tmux kill-session -t z 2>/dev/null || true; tmux new-session -d -s z -x 130 -y 34 "$ENVV target/release/zdb $1"; sleep 2.5; }
snap() { sleep 1; tmux capture-pane -t z -e -p > $OUT/$1.ansi; }
keys() { tmux send-keys -t z "$@"; }
txt() { tmux send-keys -t z -l "$1"; }

# 1. main: query + results
start shop
txt "SELECT c.name, o.status, o.total FROM orders o JOIN customers c ON c.id=o.customer_id"
keys C-r
sleep 1.5
keys C-j
keys j j j
snap main

# 2. sidebar
keys C-b
snap sidebar

# 3. palette
keys C-p
sleep 0.5
txt "ord"
snap palette
keys Escape

# 4. which-key
keys Escape
keys Space
sleep 0.6
snap whichkey
keys Escape

# 5. production
start shop:prod
txt "SELECT * FROM customers"
keys C-r
sleep 1.5
snap prod
txt ""
tmux kill-session -t z
