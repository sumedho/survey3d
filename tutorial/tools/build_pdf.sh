#!/bin/sh
# Builds tutorial/survey3d-tutorial.pdf from the markdown and its screenshots.
# Needs pandoc (brew install pandoc) and Google Chrome (or Chromium; set CHROME).
set -eu
cd "$(dirname "$0")/.."
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
mkdir -p build
pandoc survey3d-tutorial.md \
  --standalone --toc --toc-depth=2 --number-sections \
  --css tools/style.css \
  --resource-path=. \
  --embed-resources \
  --metadata pagetitle="survey3d Beginner Tutorial" \
  --variable include-before='' \
  -o build/survey3d-tutorial.html
# The cover picture goes under the title.
python3 - <<'PY'
p = "build/survey3d-tutorial.html"
s = open(p).read()
import base64
img = base64.b64encode(open("images/08-surface-3d.png", "rb").read()).decode()
s = s.replace('<p class="subtitle">', f'<img class="cover" src="data:image/png;base64,{img}"><p class="subtitle">', 1)
open(p, "w").write(s)
PY
"$CHROME" --headless=new --disable-gpu --no-pdf-header-footer --no-first-run \
  --print-to-pdf="$PWD/survey3d-tutorial.pdf" "file://$PWD/build/survey3d-tutorial.html" 2>/dev/null
echo "Wrote $PWD/survey3d-tutorial.pdf"
