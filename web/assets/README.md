# assets/

Build-time assets. Nothing here is served to the browser (that is `public/`);
these files are read by Node during `next build`.

## NotoSansJP-{Regular,Bold}.woff

Fonts for the Open Graph card images (`lib/og.tsx`). Satori — the renderer behind
`next/og` — cannot use system fonts, and its built-in font has no Japanese
glyphs, so the Japanese in the cards would render as blank boxes unless we hand
it a font. The Docker build image has no fonts installed either, so the font has
to live in the repo.

Shipping all of Noto Sans CJK JP would be ~16 MB, so these are subset to only the
characters the cards draw: ASCII, the hiragana and katakana blocks, and the kanji
in the card copy. **If you add copy with a kanji that is not already in the
subset, it renders as a blank box** — re-subset with:

```bash
# needs `fonts-noto-cjk` and python3-fonttools
python3 - <<'PY'
import os
from fontTools import subset
from fontTools.ttLib import TTFont

strings = [
    "Melon", "かざすだけ。", "支払いも、チャージも、一瞬で。",
    "専用アプリも、専用カードも不要。",
    "オンライン前払式支払手段プラットフォーム", "melon.unknowntech.jp",
    "利用規約", "加盟店規約",
]
chars = set("".join(strings))
chars |= {chr(c) for c in range(0x20, 0x7F)}      # ASCII
chars |= {chr(c) for c in range(0x3041, 0x309F)}  # hiragana
chars |= {chr(c) for c in range(0x30A0, 0x30FF)}  # katakana
chars |= set("、。「」・ー〜％円¥（）")

for weight in ["Regular", "Bold"]:
    font = TTFont(f"/usr/share/fonts/opentype/noto/NotoSansCJK-{weight}.ttc", fontNumber=0)
    opts = subset.Options()
    opts.flavor = "woff"          # satori supports ttf/otf/woff, but not woff2
    opts.desubroutinize = True
    opts.notdef_outline = True
    s = subset.Subsetter(options=opts)
    s.populate(text="".join(sorted(chars)))
    s.subset(font)
    font.flavor = "woff"
    font.save(f"assets/NotoSansJP-{weight}.woff")
    font.close()
PY
```

Noto Sans CJK JP is © The Noto Project Authors, licensed under the SIL Open Font
License 1.1, which permits subsetting and redistribution as long as the license
travels with the font — hence [OFL.txt](OFL.txt) sitting next to it.
