import type { Metadata } from "next";
import { DocPage } from "@/components/doc";
import { renderLegalDoc } from "@/lib/docs";

const TITLE = "利用規約";
const DESCRIPTION = "前払式支払手段「Melon」の利用規約。";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  openGraph: {
    title: `${TITLE} | Melon`,
    description: DESCRIPTION,
    url: "/terms",
  },
};

export default function TermsPage() {
  const { title, html } = renderLegalDoc("terms.md");
  return <DocPage title={title} html={html} />;
}
