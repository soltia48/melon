import type { MetadataRoute } from "next";
import { SITE_URL } from "@/lib/site";

/**
 * Only the landing page and the two legal documents are meant to be public. The
 * admin console and the merchant portal are login screens with nothing useful to
 * index — and the admin console is deliberately unlinked from the landing page.
 *
 * robots.txt is a request, not enforcement: a crawler that finds /admin some
 * other way may index it anyway. `X-Robots-Tag: noindex` in next.config.mjs is
 * what actually keeps those pages out of results.
 */
export default function robots(): MetadataRoute.Robots {
  return {
    rules: {
      userAgent: "*",
      allow: "/",
      // "/merchant" alone would also block /merchant-terms (robots.txt matches on
      // prefix), so the portal is excluded by its subtree plus an exact match.
      disallow: ["/admin", "/merchant$", "/merchant/"],
    },
    sitemap: `${SITE_URL}/sitemap.xml`,
  };
}
