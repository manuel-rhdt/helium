#!/bin/sh
# Downloads EB Garamond (SIL Open Font License) and installs it for the current user.
# Umbruch uses the fonts installed on the system (there is no @font-face yet).
set -eu
dest="${1:-$HOME/.fonts/umbruch-typography}"
base=https://raw.githubusercontent.com/google/fonts/main/ofl/ebgaramond
mkdir -p "$dest"
curl -fsSL -o "$dest/EBGaramond-Regular.ttf" "$base/EBGaramond%5Bwght%5D.ttf"
curl -fsSL -o "$dest/EBGaramond-Italic.ttf" "$base/EBGaramond-Italic%5Bwght%5D.ttf"
curl -fsSL -o "$dest/OFL.txt" "$base/OFL.txt"
fc-cache -f "$dest" 2>/dev/null || true
echo "installed $(ls "$dest" | wc -l) files in $dest"
