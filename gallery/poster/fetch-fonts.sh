#!/bin/sh
# Downloads the fonts that the print needs and installs them for the current user.
# Helium uses the fonts installed on the system (there is no @font-face yet).
set -eu
dest="${1:-$HOME/.fonts/helium-poster}"
base=https://raw.githubusercontent.com/notofonts/notofonts.github.io/main/fonts
mkdir -p "$dest"
for family in \
  NotoSansAdlam \
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
  NotoSerifArmenian \
  NotoSerifBalinese \
  NotoSerifBengali \
  NotoSerifDevanagari \
  NotoSerifGeorgian \
  NotoSerifGrantha \
  NotoSerifGujarati \
  NotoSerifGurmukhi \
  NotoSerifKannada \
  NotoSerifKhmer \
  NotoSerifLao \
  NotoSerifMakasar \
  NotoSerifMalayalam \
  NotoSerifMyanmar \
  NotoSerifOriya \
  NotoSerifSinhala \
  NotoSerifTamil \
  NotoSerifTelugu \
  NotoSerifThai \
  NotoSerifTibetan \
  NotoSerifToto
do
  curl -fsSL -o "$dest/$family-Regular.ttf" "$base/$family/hinted/ttf/$family-Regular.ttf"
done
# Amiri and Abyssinica SIL (SIL OFL) and Noto Serif for the CJK scripts
curl -fsSL -o "$dest/Amiri-Regular.ttf" https://raw.githubusercontent.com/google/fonts/main/ofl/amiri/Amiri-Regular.ttf
curl -fsSL -o "$dest/AbyssinicaSIL-Regular.ttf" https://raw.githubusercontent.com/google/fonts/main/ofl/abyssinicasil/AbyssinicaSIL-Regular.ttf
curl -fsSL -o "$dest/NotoSerifTC-Regular.otf" https://raw.githubusercontent.com/notofonts/noto-cjk/main/Serif/SubsetOTF/TC/NotoSerifTC-Regular.otf
curl -fsSL -o "$dest/NotoSerifJP-Regular.otf" https://raw.githubusercontent.com/notofonts/noto-cjk/main/Serif/SubsetOTF/JP/NotoSerifJP-Regular.otf
curl -fsSL -o "$dest/NotoSerifKR-Regular.otf" https://raw.githubusercontent.com/notofonts/noto-cjk/main/Serif/SubsetOTF/KR/NotoSerifKR-Regular.otf
fc-cache -f "$dest" 2>/dev/null || true
echo "installed $(ls "$dest" | wc -l) fonts in $dest"
