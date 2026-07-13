import { ogCard, OG_SIZE, OG_CONTENT_TYPE } from "@/lib/og";

export const alt = "Melon — かざすだけ。専用アプリも、専用カードも不要。";
export const size = OG_SIZE;
export const contentType = OG_CONTENT_TYPE;

export default function Image() {
  return ogCard(
    ["かざすだけ。", "支払いも、チャージも、一瞬で。"],
    "専用アプリも、専用カードも不要。",
  );
}
