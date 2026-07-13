import type { Metadata } from "next";
import { DocPage } from "@/components/doc";
import { renderLegalDoc } from "@/lib/docs";

export const metadata: Metadata = {
  title: "利用規約 | Melon",
  description: "前払式支払手段「Melon」の利用規約。",
};

export default function TermsPage() {
  const { title, html } = renderLegalDoc("terms.md");
  return <DocPage title={title} html={html} />;
}
