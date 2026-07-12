import { NextRequest, NextResponse } from "next/server";

// Same-origin API proxy.
//
// The browser only ever talks to this front-end's own origin. Requests to
// /v1/* and /healthz are rewritten to the melon-server API, so the session
// cookie (HttpOnly, SameSite=Strict) is sent and set as a first-party cookie —
// no CORS, no cookie changes on the API side. MELON_API_ORIGIN is read here at
// request time, so a single build/image works against any environment.

export const config = { matcher: ["/v1/:path*", "/healthz"] };

export function middleware(req: NextRequest) {
  const origin = process.env.MELON_API_ORIGIN || "http://127.0.0.1:8080";
  const target = new URL(req.nextUrl.pathname + req.nextUrl.search, origin);
  return NextResponse.rewrite(target);
}
