#!/bin/bash
# Copy PRIVACY.md and TERMS.md into site/ (GitHub Pages, Jekyll) without the draft note.
cd "$(dirname "$0")/.."
for f in PRIVACY:privacy:Privacy\ Policy TERMS:terms:Terms\ of\ Use; do
  IFS=: read src dst title <<< "$f"
  { printf -- '---\ntitle: %s\npermalink: /%s/\n---\n\n' "$title" "$dst"
    grep -v '^>' "$src.md" | sed -e 's#](LICENSE)#](https://opensource.org/license/mit)#' -e 's#](PRIVACY.md)#](../privacy/)#'; } > "site/$dst.md"
done
