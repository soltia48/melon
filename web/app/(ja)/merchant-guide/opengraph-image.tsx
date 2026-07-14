import { ogCard, OG_SIZE, OG_CONTENT_TYPE } from "@/lib/og";

export const alt = "Melon 加盟店オペレーションマニュアル";
export const size = OG_SIZE;
export const contentType = OG_CONTENT_TYPE;

export default function Image() {
  // Every glyph here must be in the subset font — see assets/README.md.
  return ogCard(
    ["加盟店オペレーション", "マニュアル"],
    "オンライン前払式支払手段プラットフォーム",
  );
}
