#!/bin/sh
# Downloads the Noto fonts that the print needs and installs them for the current user.
# Helium uses the fonts installed on the system (there is no @font-face yet).
set -eu
dest="${1:-$HOME/.fonts/helium-poster}"
base=https://raw.githubusercontent.com/notofonts/notofonts.github.io/main/fonts
mkdir -p "$dest"
for family in \
  NotoSansAdlam \
  NotoSansBrahmi \
  NotoSansCanadianAboriginal \
  NotoSansChakma \
  NotoSansCherokee \
  NotoSansCuneiform \
  NotoSansEgyptianHieroglyphs \
  NotoSansGothic \
  NotoSansGrantha \
  NotoSansJavanese \
  NotoSansLinearB \
  NotoSansMayanNumerals \
  NotoSansMongolian \
  NotoSansOgham \
  NotoSansOldPersian \
  NotoSansPhoenician \
  NotoSansRunic \
  NotoSansTagalog \
  NotoSansTaiTham \
  NotoSansTaiViet \
  NotoSansTifinagh \
  NotoSansUgaritic \
  NotoSansVai \
  NotoSansYi \
  NotoSerifTibetan
do
  curl -fsSL -o "$dest/$family-Regular.ttf" "$base/$family/hinted/ttf/$family-Regular.ttf"
done
fc-cache -f "$dest" 2>/dev/null || true
echo "installed $(ls "$dest" | wc -l) fonts in $dest"
