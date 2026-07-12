# 🍈 Melon

**Melon** は、FeliCa カードの **IDi(発行 ID)** をアカウントキーとする、**オンライン前払式支払手段(第三者型)** の実装です。残高はサーバ(PostgreSQL)側で一元管理し、カードは本人性・実在性(card-present)の証明に用います。

日本の**資金決済法**対策として、各チャージ(= 発行)から **6ヶ月で残高を失効**させ、「有効期間が発行日から 6ヶ月以内」の前払式支払手段に対する適用除外を狙う設計です。

> ⚠️ **免責**: 本リポジトリ中の資金決済法に関する記述は実装の意図を説明するものであり、法的助言ではありません。運用前に必ず法務レビューを行ってください。

## 主な特徴

- **オンライン相互認証** — サーバが秘密鍵を保持し FeliCa 相互認証を駆動。端末はフレームを中継するだけで、サーバ自身が**検証済み IDi** を得ます(加盟店は IDi を詐称できない)。1 認証 = 1 金銭操作に束縛。
- **複合アカウントキー** — アカウントは `(System Code, IDm, IDi)` の三つ組で識別。加盟店には生の識別子を見せず、**加盟店ごとに異なる仮名 ID** のみを返します。
- **不変台帳** — すべての残高変動は追記専用の台帳(`ledger_entries`、DB レベルで UPDATE/DELETE 禁止)に記録し、残高はそこから導出。
- **6ヶ月失効** — チャージ単位のバケットが JST 暦で 6ヶ月後に失効。消費は期限が近い順。
- **二重支払い防止** — 支払いは 1 トランザクション + 行ロックで、並行時も過剰支出なし。冪等キー対応。
- **加盟店会計 / 発行者残高** — 精算残高・決済手数料・与信限度・返金/取消と、発行者の収益残高(手数料収入 + 失効益 + 補正)を管理。
- **ユーザー認証** — 管理者・加盟店スタッフのサインオン(Argon2id、HttpOnly Cookie セッション)。

## 構成

3 つのデプロイ単位から成ります。

| コンポーネント | 内容 |
|---|---|
| **API サーバ**(`crates/melon-server`) | axum の純粋な JSON API。相互認証 + 決済 API + 失効スイープ。HTML は配信しません。 |
| **フロントエンド**(`web/`) | React / Next.js。発行者向け**管理画面**(`/admin`)と**加盟店ポータル**(`/merchant`)。`/v1` を同一オリジンで API へプロキシ。 |
| **端末**(`crates/melon-terminal`) | PaSoRi(Sony RC-S380 等)を駆動する加盟店端末。既定は**ローカル Web UI キオスク**、`--op`/`--amount` 指定で **CLI 一発実行**。 |

Rust ワークスペースのクレート:

| クレート | 役割 |
|---|---|
| `melon-core` | 純粋ドメイン(金額 `Yen`、`Idi`/`Idm`、不変台帳、6ヶ月失効ロジック)。I/O なし。 |
| `melon-db` | PostgreSQL 永続化(sqlx)。マイグレーション、口座・金銭操作、二重支払い防止。 |
| `melon-auth` | オンライン FeliCa 暗号オラクル(`felica-rs` を rusb なしで取り込み)。鍵保持・相互認証セッション。 |
| `melon-server` | axum HTTP JSON API。上記を統合し、失効スイープを常駐実行。 |
| `melon-terminal` | PaSoRi 端末(lib + bin)。キオスク / CLI の 2 モード。 |

## 技術スタック

- **Rust**(edition 2024 / toolchain 1.97)、axum、tokio、sqlx + **PostgreSQL**、[jiff](https://docs.rs/jiff)(JST 失効境界)、reqwest、tiny_http(キオスク)、argon2、[felica-rs](https://github.com/soltia48/felica-rs)(FeliCa 暗号・USB)。
- **フロントエンド**: Next.js 15 / React 19 / TypeScript。
- **デプロイ**: Docker / Docker Compose / Cloudflare Tunnel(cloudflared)。

## クイックスタート(開発)

### 1. データベース + API サーバ

```bash
# 開発用 PostgreSQL(専用コンテナ、ホスト :5433。本番は deploy/ を参照)
docker compose up -d db

export DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon
export MELON_KEYS=keys.jsonl                 # FeliCa 秘密鍵(秘匿。コミットしない)
# 初回のみ: 管理者が居ないときだけ最初の管理者を作成
export MELON_BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export MELON_BOOTSTRAP_ADMIN_PASSWORD='<10 文字以上>'

cargo run -p melon-server                    # http://127.0.0.1:8080(migrations は起動時に自動適用)
```

### 2. フロントエンド(管理画面 / 加盟店ポータル)

```bash
cd web
cp .env.local.example .env.local             # MELON_API_ORIGIN=http://127.0.0.1:8080
npm install && npm run dev                    # http://localhost:3000
#   /admin     … 発行者(管理者)アカウントでサインイン
#   /merchant  … 加盟店アカウントでサインイン
```

セッション Cookie(HttpOnly / SameSite=Strict)を保つため、フロントが `/v1` を同一オリジンで API へプロキシします。詳細は [web/README.md](web/README.md)。

### 3. 端末(要 PaSoRi RC-S380 + カード)

```bash
# 引数なし → Web UI キオスクが起動し、既定ブラウザで UI が開く
# (API キーは画面の「⚙ 設定」から設定でき、保存されて次回自動読込)
cargo run -p melon-terminal -- --server http://127.0.0.1:8080

# CLI 一発実行(--op / --amount を指定)
cargo run -p melon-terminal -- --server http://127.0.0.1:8080 \
  --api-key <加盟店APIキー> --op pay --amount 500
```

> 既定のサーバは本番(`https://melon.unknowntech.jp`)です。ローカル検証時は上記のように `--server http://127.0.0.1:8080` を渡してください。

### テスト

```bash
DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon cargo test --workspace
```

## デプロイ

- [`Dockerfile`](Dockerfile) — `melon-server` の本番イメージ(プライベート依存 `felica-rs` を SSH で取得)。
- [`web/Dockerfile`](web/Dockerfile) — フロントエンドの本番イメージ。
- [`deploy/compose.yaml`](deploy/compose.yaml) — `server` + `web` + `cloudflared` を Cloudflare Tunnel 経由で公開。**リバースプロキシ不要・インバウンドポートなし**。手順は `deploy/` を参照。

## CI / CD(GitHub Actions)

- [`.github/workflows/ci.yml`](.github/workflows/ci.yml) — push / PR ごとに `fmt` + `clippy` + `test`(PostgreSQL サービス上で全ワークスペース)。
- [`.github/workflows/release.yml`](.github/workflows/release.yml) — `v*` タグの push で `melon-terminal` を **Linux / Windows / macOS(Intel・Apple Silicon)** 向けにビルドし、Release に添付。

いずれもプライベート依存 `felica-rs` を取得するため、リポジトリシークレット **`FELICA_RS_TOKEN`**(`soltia48/felica-rs` を読める PAT)が必要です。

## ドキュメント

| ドキュメント | 内容 |
|---|---|
| [docs/architecture.md](docs/architecture.md) | アーキテクチャ、クレート構成、認証モデル、信頼境界 |
| [docs/domain.md](docs/domain.md) | ドメイン・会計仕様(アカウント/残高/台帳/失効/手数料/与信/精算/資金決済法) |
| [docs/api.md](docs/api.md) | HTTP API リファレンス |
| [docs/operations.md](docs/operations.md) | 環境変数、DB、端末、Web UI、ビルド、テスト |
| [web/README.md](web/README.md) | フロントエンド(開発・ビルド・デプロイ) |

## ライセンス

[MIT](LICENSE) © KIRISHIKI Yudai
