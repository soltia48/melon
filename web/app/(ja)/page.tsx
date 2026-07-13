import type { Metadata } from "next";
import { Landing } from "@/components/landing";
import { LANGUAGE_ALTERNATES } from "@/lib/site";
import { LP_JA } from "@/lib/lp-copy";

export const metadata: Metadata = {
  alternates: { canonical: "/", languages: LANGUAGE_ALTERNATES },
};

export default function Home() {
  return <Landing c={LP_JA} />;
}
