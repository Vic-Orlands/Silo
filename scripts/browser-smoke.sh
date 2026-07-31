#!/usr/bin/env sh
set -eu

command -v node >/dev/null 2>&1 || { echo "node is required" >&2; exit 1; }
npm install --no-audit --no-fund --ignore-scripts
npx playwright install chromium
npm run test:browser
