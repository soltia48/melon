import { ogCard, OG_SIZE, OG_CONTENT_TYPE } from "@/lib/og";

export const alt = "Melon 加盟店規約";
export const size = OG_SIZE;
export const contentType = OG_CONTENT_TYPE;

export default function Image() {
  return ogCard(["加盟店規約"], "オンライン前払式支払手段プラットフォーム");
}
