import { ogCard, OG_SIZE, OG_CONTENT_TYPE } from "@/lib/og";

export const alt = "Melon — just tap. No app to install, no card to issue.";
export const size = OG_SIZE;
export const contentType = OG_CONTENT_TYPE;

export default function Image() {
  return ogCard(
    ["Just tap.", "Pay or top up in an instant."],
    "No app to install. No card to issue.",
  );
}
