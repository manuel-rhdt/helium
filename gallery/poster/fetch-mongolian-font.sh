#!/bin/sh
# Downloads Mongolian Title (Almas Inc.) for the Mongolian tile of the print and installs it
# for the current user. The font is not redistributed from this repository, and the print
# does not depend on it: when the download or the checksum fails, this script warns and exits
# successfully, and the tile falls back to Noto Sans Mongolian.
# Usage: fetch-mongolian-font.sh [directory]
dest="${1:-$HOME/.fonts/umbruch-poster}"
file=mngltitleotf.ttf
sha256=e84fd1b17876ac49674ff7542ea9de6b787af2a66980500cf79e92854036bb04
url="http://www.mongolfont.com/jAlmas/cms/documents/mongolfont/font/$file"

if [ -f "$dest/$file" ]; then
  exit 0
fi
mkdir -p "$dest"
tmp="$(mktemp)"
# The site is served over plain HTTP, so the checksum is what makes the download trustworthy.
if curl -fsSL --retry 2 --connect-timeout 15 --max-time 120 -o "$tmp" "$url" \
  && [ "$(sha256sum "$tmp" | cut -d' ' -f1)" = "$sha256" ]; then
  mv "$tmp" "$dest/$file"
  chmod 644 "$dest/$file"
  fc-cache -f "$dest" 2>/dev/null || true
  echo "installed $file in $dest"
else
  rm -f "$tmp"
  echo "warning: could not download $file (or its checksum differs); Mongolian falls back to Noto Sans Mongolian" >&2
fi
exit 0
