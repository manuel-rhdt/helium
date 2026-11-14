#!/bin/sh
# Downloads the Noto fonts that the poster needs and installs them for the current user.
# Helium uses the fonts installed on the system (there is no @font-face yet).
set -eu
dest="${1:-$HOME/.fonts/helium-poster}"
base=https://raw.githubusercontent.com/notofonts/notofonts.github.io/main/fonts
mkdir -p "$dest"
for family in \
  NotoSansAdlam \
  NotoSansAnatolianHieroglyphs \
  NotoSansAvestan \
  NotoSansBamum \
  NotoSansBatak \
  NotoSansBhaiksuki \
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
  NotoSansCyproMinoan \
  NotoSansDeseret \
  NotoSansDuployan \
  NotoSansEgyptianHieroglyphs \
  NotoSansElbasan \
  NotoSansElymaic \
  NotoSansGlagolitic \
  NotoSansGothic \
  NotoSansGrantha \
  NotoSansGunjalaGondi \
  NotoSansHanifiRohingya \
  NotoSansHanunoo \
  NotoSansImperialAramaic \
  NotoSansInscriptionalPahlavi \
  NotoSansJavanese \
  NotoSansKaithi \
  NotoSansKayahLi \
  NotoSansKharoshthi \
  NotoSansKhojki \
  NotoSansKhudawadi \
  NotoSansLepcha \
  NotoSansLimbu \
  NotoSansLinearA \
  NotoSansLinearB \
  NotoSansLisu \
  NotoSansLycian \
  NotoSansLydian \
  NotoSansMahajani \
  NotoSansMandaic \
  NotoSansManichaean \
  NotoSansMasaramGondi \
  NotoSansMayanNumerals \
  NotoSansMedefaidrin \
  NotoSansMeeteiMayek \
  NotoSansMendeKikakui \
  NotoSansMeroitic \
  NotoSansModi \
  NotoSansMongolian \
  NotoSansMro \
  NotoSansMultani \
  NotoSansNKo \
  NotoSansNabataean \
  NotoSansNagMundari \
  NotoSansNewTaiLue \
  NotoSansNewa \
  NotoSansNushu \
  NotoSansOgham \
  NotoSansOlChiki \
  NotoSansOldHungarian \
  NotoSansOldItalic \
  NotoSansOldNorthArabian \
  NotoSansOldPermic \
  NotoSansOldPersian \
  NotoSansOldSouthArabian \
  NotoSansOldTurkic \
  NotoSansOsage \
  NotoSansOsmanya \
  NotoSansPahawhHmong \
  NotoSansPalmyrene \
  NotoSansPauCinHau \
  NotoSansPhagsPa \
  NotoSansPhoenician \
  NotoSansPsalterPahlavi \
  NotoSansRejang \
  NotoSansRunic \
  NotoSansSamaritan \
  NotoSansSaurashtra \
  NotoSansSharada \
  NotoSansShavian \
  NotoSansSiddham \
  NotoSansSogdian \
  NotoSansSoyombo \
  NotoSansSundanese \
  NotoSansSylotiNagri \
  NotoSansTagalog \
  NotoSansTagbanwa \
  NotoSansTaiLe \
  NotoSansTaiTham \
  NotoSansTaiViet \
  NotoSansTakri \
  NotoSansTangsa \
  NotoSansTifinagh \
  NotoSansTirhuta \
  NotoSansUgaritic \
  NotoSansVai \
  NotoSansWancho \
  NotoSansWarangCiti \
  NotoSansYi \
  NotoSansZanabazarSquare \
  NotoSerifAhom \
  NotoSerifBalinese \
  NotoSerifDivesAkuru \
  NotoSerifDogra \
  NotoSerifKhitanSmallScript \
  NotoSerifMakasar \
  NotoSerifTangut \
  NotoSerifTibetan \
  NotoSerifToto \
  NotoSerifVithkuqi
do
  curl -fsSL -o "$dest/$family-Regular.ttf" "$base/$family/hinted/ttf/$family-Regular.ttf"
done
fc-cache -f "$dest" 2>/dev/null || true
echo "installed $(ls "$dest" | wc -l) fonts in $dest"
