#!/bin/bash
# Rebuild vendor/codemirror/codemirror.min.js (the Tauri app's editor) from npm.
set -euo pipefail
cd "$(dirname "$0")/codemirror"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp entry.js "$work/"
cd "$work"
npm init -y >/dev/null
npm install --silent --no-audit --no-fund esbuild @codemirror/view @codemirror/state \
    @codemirror/commands @codemirror/language @codemirror/search @codemirror/lang-markdown \
    @codemirror/language-data @codemirror/theme-one-dark @lezer/highlight
npx esbuild entry.js --bundle --minify --format=iife --global-name=CM --legal-comments=eof \
    --outfile=codemirror.min.js
node -e 'const p=require("./package-lock.json").packages; const v=Object.entries(p).filter(([k])=>k.startsWith("node_modules/@codemirror/")).map(([k,x])=>k.slice(13)+" "+x.version); console.log(v.join("\n"))' > VERSIONS
cp codemirror.min.js VERSIONS "$OLDPWD/"
curl -sSf -o "$OLDPWD/LICENSE" https://raw.githubusercontent.com/codemirror/dev/main/LICENSE
