import type { Metadata } from "next";
import { DocPage } from "@/components/doc";
import { renderDoc } from "@/lib/docs";

const TITLE = "加盟店規約";
const DESCRIPTION = "前払式支払手段「Melon」の加盟店規約。";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  openGraph: {
    title: `${TITLE} | Melon`,
    description: DESCRIPTION,
    url: "/merchant-terms",
  },
};

export default function MerchantTermsPage() {
  const { title, html } = renderDoc("merchant-terms.md");
  return <DocPage title={title} html={html} />;
}
