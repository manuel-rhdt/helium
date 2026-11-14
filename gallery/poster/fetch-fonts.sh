#!/bin/sh
# Downloads the fonts that the print needs and installs them for the current user.
# Helium uses the fonts installed on the system (there is no @font-face yet).
set -eu
dest="${1:-$HOME/.fonts/helium-poster}"
base=https://raw.githubusercontent.com/notofonts/notofonts.github.io/main/fonts
mkdir -p "$dest"
for family in \
  NotoSansAdlam \
  NotoSansAnatolianHieroglyphs \
  NotoSansAvestan \
  NotoSansBatak \
  NotoSansBrahmi \
  NotoSansBuginese \
  NotoSansBuhid \
  NotoSansCanadianAboriginal \
  NotoSansCarian \
  NotoSansCaucasianAlbanian \
  NotoSansChakma \
  NotoSansCham \
  NotoSansCherokee \
  NotoSansCoptic \
  NotoSansCuneiform \
  NotoSansCypriot \
  NotoSansDeseret \
  NotoSansEgyptianHieroglyphs \
  NotoSansElbasan \
  NotoSansGlagolitic \
  NotoSansGothic \
  NotoSansGunjalaGondi \
  NotoSansHanifiRohingya \
  NotoSansHanunoo \
  NotoSansInscriptionalPahlavi \
  NotoSansJavanese \
  NotoSansKaithi \
  NotoSansKayahLi \
  NotoSansKharoshthi \
  NotoSansKhudawadi \
  NotoSansLepcha \
  NotoSansLimbu \
  NotoSansLinearA \
  NotoSansLinearB \
  NotoSansLisu \
  NotoSansLycian \
  NotoSansLydian \
  NotoSansMandaic \
  NotoSansManichaean \
  NotoSansMasaramGondi \
  NotoSansMayanNumerals \
  NotoSansMedefaidrin \
  NotoSansMeeteiMayek \
  NotoSansMendeKikakui \
  NotoSansModi \
  NotoSansMongolian \
  NotoSansMro \
  NotoSansNKo \
  NotoSansNagMundari \
  NotoSansNewTaiLue \
  NotoSansNewa \
  NotoSansOgham \
  NotoSansOlChiki \
  NotoSansOldHungarian \
  NotoSansOldItalic \
  NotoSansOldPermic \
  NotoSansOldPersian \
  NotoSansOldTurkic \
  NotoSansOsage \
  NotoSansOsmanya \
  NotoSansPahawhHmong \
  NotoSansPhagsPa \
  NotoSansPhoenician \
  NotoSansRejang \
  NotoSansRunic \
  NotoSansSaurashtra \
  NotoSansSharada \
  NotoSansShavian \
  NotoSansSogdian \
  NotoSansSoyombo \
  NotoSansSundanese \
  NotoSansSylotiNagri \
  NotoSansTagalog \
  NotoSansTagbanwa \
  NotoSansTaiLe \
  NotoSansTaiTham \
  NotoSansTaiViet \
  NotoSansTangsa \
  NotoSansTifinagh \
  NotoSansTirhuta \
  NotoSansUgaritic \
  NotoSansVai \
  NotoSansWancho \
  NotoSansWarangCiti \
  NotoSansYi \
  NotoSerifAhom \
  NotoSerifArmenian \
  NotoSerifBalinese \
  NotoSerifBengali \
  NotoSerifDevanagari \
  NotoSerifDivesAkuru \
  NotoSerifDogra \
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
