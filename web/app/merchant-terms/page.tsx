import type { Metadata } from "next";
import { DocPage } from "@/components/doc";
import { renderLegalDoc } from "@/lib/docs";

export const metadata: Metadata = {
  title: "加盟店規約 | Melon",
  description: "前払式支払手段「Melon」の加盟店規約。",
};

export default function MerchantTermsPage() {
  const { title, html } = renderLegalDoc("merchant-terms.md");
  return <DocPage title={title} html={html} />;
}
