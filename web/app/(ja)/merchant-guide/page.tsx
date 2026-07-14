import type { Metadata } from "next";
import { DocPage } from "@/components/doc";
import { renderDoc } from "@/lib/docs";

const TITLE = "加盟店オペレーションマニュアル";
const DESCRIPTION =
  "Melon をお店で取り扱うための手引き。レジでの操作とエラー対応、端末と API キーの管理、精算の考え方。";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  openGraph: {
    title: `${TITLE} | Melon`,
    description: DESCRIPTION,
    url: "/merchant-guide",
  },
};

export default function MerchantGuidePage() {
  const { title, html } = renderDoc("merchant-guide.md");
  return <DocPage title={title} html={html} />;
}
