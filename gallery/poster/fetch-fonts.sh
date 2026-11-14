#!/bin/sh
# Downloads the Noto fonts that the print needs and installs them for the current user.
# Helium uses the fonts installed on the system (there is no @font-face yet).
set -eu
dest="${1:-$HOME/.fonts/helium-poster}"
base=https://raw.githubusercontent.com/notofonts/notofonts.github.io/main/fonts
mkdir -p "$dest"
for family in \
  NotoSansAdlam \
  NotoSansBalinese \
  NotoSansBatak \
  NotoSansBrahmi \
  NotoSansBuginese \
  NotoSansBuhid \
  NotoSansCanadianAboriginal \
  NotoSansChakma \
  NotoSansCham \
  NotoSansCherokee \
  NotoSansCuneiform \
  NotoSansDeseret \
  NotoSansEgyptianHieroglyphs \
  NotoSansGothic \
  NotoSansGrantha \
  NotoSansHanifiRohingya \
  NotoSansHanunoo \
  NotoSansJavanese \
  NotoSansKayahLi \
  NotoSansLepcha \
  NotoSansLimbu \
  NotoSansLinearB \
  NotoSansLisu \
  NotoSansMayanNumerals \
  NotoSansMedefaidrin \
  NotoSansMeeteiMayek \
  NotoSansMendeKikakui \
  NotoSansMongolian \
  NotoSansMro \
  NotoSansNKo \
  NotoSansNagMundari \
  NotoSansNewTaiLue \
  NotoSansOgham \
  NotoSansOlChiki \
  NotoSansOldPersian \
  NotoSansOsage \
  NotoSansOsmanya \
  NotoSansPahawhHmong \
  NotoSansPhoenician \
  NotoSansRejang \
  NotoSansRunic \
  NotoSansSaurashtra \
  NotoSansShavian \
  NotoSansSundanese \
  NotoSansSylotiNagri \
  NotoSansTagalog \
  NotoSansTagbanwa \
  NotoSansTaiLe \
  NotoSansTaiTham \
  NotoSansTaiViet \
  NotoSansTangsa \
  NotoSansTifinagh \
  NotoSansUgaritic \
  NotoSansVai \
  NotoSansWancho \
  NotoSansWarangCiti \
  NotoSansYi \
  NotoSerifMakasar \
  NotoSerifTibetan \
  NotoSerifToto
do
  curl -fsSL -o "$dest/$family-Regular.ttf" "$base/$family/hinted/ttf/$family-Regular.ttf"
done
fc-cache -f "$dest" 2>/dev/null || true
echo "installed $(ls "$dest" | wc -l) fonts in $dest"
