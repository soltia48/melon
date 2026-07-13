// @ts-check

/**
 * The front-end never talks to the API cross-origin. `middleware.ts` proxies
 * /v1/* and /healthz to the API under the SAME origin the browser loaded the app
 * from, so the melon-server session cookie (HttpOnly, SameSite=Strict) keeps
 * flowing untouched — the API needs no CORS and no cookie changes. The API
 * address (MELON_API_ORIGIN) is read at request time, so one build runs against
 * any environment.
 *
 * `output: "standalone"` emits a self-contained server bundle for a small Docker
 * image (see Dockerfile).
 */

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  output: "standalone",
  async headers() {
    // Baseline hardening for the HTML the front-end serves. (For /v1 the API's
    // own security headers pass through the proxy; HSTS is applied at the
    // Cloudflare edge in production.)
    const noindex = [{ key: "X-Robots-Tag", value: "noindex, nofollow" }];
    return [
      {
        source: "/:path*",
        headers: [
          { key: "X-Frame-Options", value: "DENY" },
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "Referrer-Policy", value: "no-referrer" },
        ],
      },
      // The consoles are login screens with nothing to index, and /admin is kept
      // unlinked on purpose. robots.txt only asks; this tells a crawler that
      // reached the page anyway to keep it out of the index. Note the portal is
      // matched exactly and by subtree so that /merchant-terms — a public page —
      // is not caught by the prefix.
      { source: "/admin", headers: noindex },
      { source: "/admin/:path*", headers: noindex },
      { source: "/merchant", headers: noindex },
      { source: "/merchant/:path*", headers: noindex },
    ];
  },
};

export default nextConfig;
