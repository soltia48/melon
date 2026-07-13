#!/usr/bin/env bash
# Create deploy/secrets/ with strong generated credentials.
#
#   cd deploy && ./init-secrets.sh
#
# It never overwrites an existing file, so it is safe to re-run. You must still
# place the FeliCa key file yourself — it cannot be generated.
set -euo pipefail
cd "$(dirname "$0")"

mkdir -p secrets
chmod 700 secrets   # the directory is what protects the secrets on the host

write_if_absent() {
  local path="$1" value="$2"
  if [[ -e "$path" ]]; then
    echo "  keep    $path (already exists)"
    return
  fi
  printf '%s' "$value" > "$path"
  # 444: readable by whatever uid the container runs as; the 700 dir guards the host.
  chmod 444 "$path"
  echo "  created $path"
}

DB_PASSWORD="$(openssl rand -base64 33 | tr -d '\n/+=' | cut -c1-40)"
ADMIN_PASSWORD="$(openssl rand -base64 33 | tr -d '\n/+=' | cut -c1-32)"

write_if_absent secrets/db_password "$DB_PASSWORD"
# `db` is the compose service name; 5432 is the in-network port.
write_if_absent secrets/database_url "postgres://melon:${DB_PASSWORD}@db:5432/melon"
write_if_absent secrets/bootstrap_admin_password "$ADMIN_PASSWORD"
# Turnstile is OPTIONAL. Compose requires the secret file to exist, so create it
# EMPTY — the server reads an empty secret as "challenge disabled". Paste the
# secret key in (and set MELON_TURNSTILE_SITE_KEY in .env) to turn it on.
write_if_absent secrets/turnstile_secret ""

echo
# Two things cannot be generated — they must be supplied.
if [[ -e secrets/keys.jsonl ]]; then
  echo "  keep    secrets/keys.jsonl"
else
  echo "  MISSING secrets/keys.jsonl — copy your FeliCa DES key file there:"
  echo "            cp /path/to/keys.jsonl secrets/ && chmod 444 secrets/keys.jsonl"
  echo "          Without it the server cannot authenticate any card."
fi
# The Cloudflare tunnel token is the one secret that must live in .env (the
# distroless cloudflared image cannot read it from a file).
if grep -qE '^CLOUDFLARE_TUNNEL_TOKEN=.+' .env 2>/dev/null; then
  echo "  keep    .env CLOUDFLARE_TUNNEL_TOKEN"
else
  echo "  MISSING CLOUDFLARE_TUNNEL_TOKEN in deploy/.env — create the tunnel:"
  echo "            Zero Trust → Networks → Tunnels → Create a tunnel → Cloudflared"
  echo "            copy the token into deploy/.env, then chmod 600 .env"
  echo "            Public hostname: <your-host> → HTTP → server:8080"
  echo "          (service must be server:8080 — the compose service, not localhost)"
fi
# Turnstile is optional — say whether the sign-in challenge is on.
if [[ -s secrets/turnstile_secret ]] && grep -qE '^MELON_TURNSTILE_SITE_KEY=.+' .env 2>/dev/null; then
  echo "  keep    Turnstile ENABLED on sign-in (site key in .env + secret file)"
else
  echo "  off     Turnstile sign-in challenge is DISABLED (optional). To enable:"
  echo "            Cloudflare → Turnstile → Add widget (hostname = your public host)"
  echo "            site key   → MELON_TURNSTILE_SITE_KEY in deploy/.env"
  echo "            secret key → deploy/secrets/turnstile_secret (chmod 444)"
fi

echo
echo "First sign-in (write these down; the password file is the only copy):"
echo "  URL      https://<the public hostname you set on the Cloudflare tunnel>/admin"
echo "  email    \$MELON_BOOTSTRAP_ADMIN_EMAIL (from deploy/.env)"
echo "  password $(cat secrets/bootstrap_admin_password)"
echo
echo "The database password and DATABASE_URL must stay in sync — if you rotate one,"
echo "rotate both (and ALTER USER melon WITH PASSWORD in Postgres)."
