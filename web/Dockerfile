# syntax=docker/dockerfile:1
#
# melon-web — production image for the React/Next.js front-end.
#
# A plain public front-end: no private deps, no SSH needed at build time.
#
#   docker build -t melon-web web/
#
# At runtime, set MELON_API_ORIGIN to the API's address (read per request by
# middleware.ts, so one image serves any environment).

# ---------- deps ----------
FROM node:22-bookworm-slim AS deps
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci

# ---------- builder ----------
FROM node:22-bookworm-slim AS builder
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .
ENV NEXT_TELEMETRY_DISABLED=1
RUN npm run build

# ---------- runtime ----------
FROM node:22-bookworm-slim AS runner
WORKDIR /app
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0

RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin nextjs

# `output: "standalone"` bundles the server and only the node_modules it needs.
# It does NOT include `public/` or `.next/static` — copy both in explicitly, or
# static assets (e.g. /melon-logo.png) 404 at runtime.
COPY --from=builder /app/.next/standalone ./
COPY --from=builder /app/.next/static ./.next/static
COPY --from=builder /app/public ./public

USER nextjs
EXPOSE 3000

# The landing page is static (does not depend on the API), so it makes a clean
# liveness probe. Node 22 has a global fetch().
HEALTHCHECK --interval=15s --timeout=3s --start-period=15s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:3000/').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

CMD ["node", "server.js"]
