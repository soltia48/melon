# Melon 仕様書

**Melon** は、FeliCa カードの **IDi(発行 ID)** をアカウントキーとする、**オンライン前払式支払手段(第三者型)** のサーバです。残高はサーバ(PostgreSQL)側で一元管理し、カードは「本人性・実在性(card-present)の証明」に用います。

日本の**資金決済法**対策として、各チャージ(= 発行)から **6ヶ月で残高を失効**させ、「有効期間が発行日から 6ヶ月以内」の前払式支払手段に対する適用除外を狙う設計です。

> ⚠️ 本ドキュメント中の資金決済法に関する記述は実装の意図を説明するものであり、法的助言ではありません。運用前に必ず法務レビューを行ってください。

## 主要な特徴

- **オンライン相互認証**: サーバが秘密鍵を保持し FeliCa 相互認証を駆動。端末はフレームを中継するだけで、サーバ自身が検証済み IDi を得る(加盟店は IDi を詐称できない)。
- **複合アカウントキー**: アカウントは `(System Code, IDm, IDi)` の三つ組で識別(IDi はシステム内でのみ一意)。
- **不変台帳**: すべての残高変動は追記専用の台帳(`ledger_entries`)に記録。残高はそこから導出。
- **6ヶ月失効**: チャージ単位のバケットが 6ヶ月(JST 暦)で失効。期限が近い順に消費。
- **二重支払い防止**: 支払いは 1 トランザクション + 行ロックで、並行時も過剰支出しない。冪等キー対応。
- **加盟店会計**: 精算残高・決済手数料・与信限度・返金/取消を管理。
- **発行者残高**: 決済手数料収入 + 消滅済み残高(失効益) + 引き出し・補正の会計上の収益残高を管理。
- **Web 管理画面 / 加盟店ポータル**(別アプリ `web/`, React/Next.js)、**PaSoRi 端末**(引数なしでローカル Web UI キオスク、`--op`/`--amount` 指定で CLI 一発実行)を同梱。

## ドキュメント一覧

| ドキュメント | 内容 |
|---|---|
| [architecture.md](architecture.md) | アーキテクチャ、クレート構成、認証モデル、信頼境界 |
| [domain.md](domain.md) | ドメイン・会計仕様(アカウント/残高/台帳/失効/手数料/与信/精算/資金決済法) |
| [api.md](api.md) | HTTP API リファレンス |
| [operations.md](operations.md) | 環境変数、DB、端末、Web UI、ビルド、テスト |

規約類(利用規約・加盟店規約)は Web に掲載するため [`web/content/`](../web/content/) に置いています。Markdown が唯一の原本で、ビルド時に `/terms` と `/merchant-terms` へ静的レンダリングされます。

## クイックスタート

```bash
# 1. 開発用 PostgreSQL(専用コンテナ, ホスト :5433。直下 compose.yaml = 開発専用。本番は deploy/compose.yaml)
docker compose up -d db

# 2. サーバ起動(migrations は起動時に自動適用)
export DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon
export MELON_KEYS=keys.jsonl            # FeliCa 鍵(秘匿。リポジトリには含めない)
# 初回のみ: 最初の管理者ユーザーを作成(管理者が居ない時だけ作成される)
export MELON_BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export MELON_BOOTSTRAP_ADMIN_PASSWORD='<10 文字以上>'
cargo run -p melon-server

# 3. フロントエンド(別アプリ web/, React/Next.js。メール + パスワードでサインイン)
cd web && cp .env.local.example .env.local && npm install && npm run dev   # http://localhost:3000
#    /admin     … 発行者(管理者)アカウント
#    /merchant  … 加盟店アカウント

# 4. 端末(要 PaSoRi RC-S380 + カード)。引数なしでキオスク、--op/--amount で CLI 一発実行
cargo run -p melon-terminal -- --server http://127.0.0.1:8080 \
  --api-key <加盟店APIキー> --op pay --amount 500
```

詳細は各ドキュメントを参照してください。全ワークスペースのテストは `DATABASE_URL=... cargo test --workspace` で実行できます。
