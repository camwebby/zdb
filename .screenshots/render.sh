#!/bin/sh
# Render .screenshots/out/*.ansi to docs/screenshots/*.png via headless Chrome.
set -e
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
mkdir -p docs/screenshots .screenshots/html
for f in main sidebar palette whichkey prod; do
  python3 .screenshots/ansi2html.py .screenshots/out/$f.ansi .screenshots/html/$f.html "zdb"
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 \
    --force-device-scale-factor=2 --window-size=1214,770 \
    --screenshot="$PWD/docs/screenshots/$f.png" "file://$PWD/.screenshots/html/$f.html" 2>/dev/null
done
ls -la docs/screenshots
