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

echo
if [[ -e secrets/keys.jsonl ]]; then
  echo "  keep    secrets/keys.jsonl"
else
  echo "  MISSING secrets/keys.jsonl — copy your FeliCa DES key file there:"
  echo "            cp /path/to/keys.jsonl secrets/ && chmod 444 secrets/keys.jsonl"
  echo "          Without it the server cannot authenticate any card."
fi

echo
echo "First sign-in (write these down; the password file is the only copy):"
echo "  URL      https://<MELON_DOMAIN>/admin"
echo "  email    \$MELON_BOOTSTRAP_ADMIN_EMAIL (from deploy/.env)"
echo "  password $(cat secrets/bootstrap_admin_password)"
echo
echo "The database password and DATABASE_URL must stay in sync — if you rotate one,"
echo "rotate both (and ALTER USER melon WITH PASSWORD in Postgres)."
