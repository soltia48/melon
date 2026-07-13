import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/site";

/** The public surface: the landing page (two languages) and the legal documents. */
export default function sitemap(): MetadataRoute.Sitemap {
  const languages = { ja: SITE_URL, en: `${SITE_URL}/en` };
  return [
    {
      url: SITE_URL,
      changeFrequency: "monthly",
      priority: 1,
      alternates: { languages },
    },
    {
      url: `${SITE_URL}/en`,
      changeFrequency: "monthly",
      priority: 0.8,
      alternates: { languages },
    },
    { url: `${SITE_URL}/terms`, changeFrequency: "yearly", priority: 0.5 },
    {
      url: `${SITE_URL}/merchant-terms`,
      changeFrequency: "yearly",
      priority: 0.5,
    },
  ];
}
