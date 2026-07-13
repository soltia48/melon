import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/site";

/** The public surface: the landing page and the two legal documents. */
export default function sitemap(): MetadataRoute.Sitemap {
  return [
    { url: SITE_URL, changeFrequency: "monthly", priority: 1 },
    { url: `${SITE_URL}/terms`, changeFrequency: "yearly", priority: 0.5 },
    {
      url: `${SITE_URL}/merchant-terms`,
      changeFrequency: "yearly",
      priority: 0.5,
    },
  ];
}
