import type { Metadata } from "next";
import { Landing } from "@/components/landing";
import { LANGUAGE_ALTERNATES } from "@/lib/site";
import { LP_EN } from "@/lib/lp-copy";

export const metadata: Metadata = {
  alternates: { canonical: "/en", languages: LANGUAGE_ALTERNATES },
};

export default function HomeEn() {
  return <Landing c={LP_EN} />;
}
