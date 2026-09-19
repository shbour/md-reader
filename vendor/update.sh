#!/bin/bash
# Re-download the bundled JS libraries and rebuild KaTeX's self-contained CSS.
# Usage: vendor/update.sh [katex-version] [mermaid-version]
set -euo pipefail
cd "$(dirname "$0")"
KV=${1:-0.18.7}
MV=${2:-12.0.0}
CDN=https://cdn.jsdelivr.net/npm

rm -rf katex mermaid && mkdir -p katex/fonts mermaid
curl -sSf -o katex/katex.min.js "$CDN/katex@$KV/dist/katex.min.js"
curl -sSf -o katex/katex.min.css "$CDN/katex@$KV/dist/katex.min.css"
curl -sSf -o katex/LICENSE "$CDN/katex@$KV/LICENSE"
for f in $(grep -o 'fonts/[A-Za-z0-9_-]*\.woff2' katex/katex.min.css | sort -u); do
    curl -sSf -o "katex/$f" "$CDN/katex@$KV/dist/$f"
done
curl -sSf -o mermaid/mermaid.min.js "$CDN/mermaid@$MV/dist/mermaid.min.js"
curl -sSf -o mermaid/LICENSE "$CDN/mermaid@$MV/LICENSE"

# The preview page has no file access to these fonts, so inline them as
# data: URIs and drop the woff/ttf fallbacks WebKit never needs.
python3 - <<'PY'
import base64, re
css = open("katex/katex.min.css").read()
def inline(m):
    data = base64.b64encode(open("katex/" + m.group(1), "rb").read()).decode()
    return f'src:url(data:font/woff2;base64,{data}) format("woff2")'
css, n = re.subn(r'src:url\((fonts/[^)]+\.woff2)\) format\("woff2"\)(?:,url\([^)]+\) format\("[a-z]+"\))*', inline, css)
assert n > 0 and "url(fonts/" not in css, "unexpected katex.min.css layout"
open("katex/katex.inline.css", "w").write(css)
print(f"inlined {n} fonts")
PY
echo "$KV" > katex/VERSION
echo "$MV" > mermaid/VERSION
