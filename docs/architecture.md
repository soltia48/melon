# アーキテクチャ

## 全体像

Melon は「**オンライン前払式支払手段**」です。残高は **PostgreSQL 側**で一元管理し、FeliCa カードは**認証(検証済み IDi の取得)専用**に使います。オンチップ残高の読み書きは行いません。

決済フロー(概念):

```
  加盟店端末(PaSoRi)            melon-server(鍵を保持)         PostgreSQL
  ─────────────────            ───────────────────           ──────────
  カードをポーリング → IDm/PMm
     │  POST /v1/mutual-authentication ─────▶ 鍵導出・Auth1 フレーム生成
     │  ◀──────────── { command.frame } ─────┘
  フレームをカードへ送信
     │  POST … { card_response } ───────────▶ 検証・Auth2 …
     │  ◀──────────── { …complete, issue_id }┘  ← サーバが検証済み IDi を取得
     │                                          セッションに (system_code, idm, idi) を束縛
     │  POST /v1/payments { session_id, amount } ▶ 残高操作(1 認証 = 1 金銭操作) ──▶ 台帳・バケット更新
     │  ◀──────────────────── 決済結果 ────────┘
```

**信頼モデル**: サーバが秘密 DES 鍵を保持し相互認証を駆動するため、IDi は**サーバ検証済み・card-present 証明済み**です。加盟店は鍵を持たず IDi を詐称できません。加盟店認証(API キー)は別レイヤで、どの加盟店の取引かの認可・精算に使います。

## クレート構成(Cargo ワークスペース)

| クレート | 役割 | 主な依存 |
|---|---|---|
| **melon-core** | 純粋ドメイン(I/O なし)。`Yen`/`PositiveYen`、`Idi`、`AccountKey`、台帳・失効の型、6ヶ月失効計算(jiff)、消費アルゴリズム、手数料計算 | jiff, serde, uuid |
| **melon-db** | PostgreSQL 永続化(sqlx)。マイグレーション、口座・加盟店・金銭操作(top_up/pay/refund/void/adjust/sweep/report)、二重支払い防止 | sqlx, melon-core, time |
| **melon-auth** | FeliCa 暗号オラクル(`felica-auth-server` を rusb なしで取り込み)。鍵ストア、相互認証セッション、リレードライバ | felica-rs(`default-features=false`), axum, flume |
| **melon-server** | axum HTTP サーバ(純粋な JSON API)。相互認証 + 決済 API + 加盟店/管理 API + 失効スイープを統合。フロントエンドは別アプリ(`web/`) | melon-core/db/auth, axum, sqlx, sha2 |
| **melon-terminal** | 加盟店端末(lib+bin)。PaSoRi を駆動しフレームを中継。既定は Web UI キオスク、`--op`/`--amount` 指定で CLI 一発実行の 2 モード | felica-rs(`features=["usb"]`), reqwest, tiny_http |

### レイヤリング

```
melon-terminal ──HTTP──▶ melon-server ──┬─▶ melon-auth  ──▶ felica-rs(felica_standard, rusb なし)
                                        ├─▶ melon-db    ──▶ PostgreSQL
                                        └─▶ melon-core(純粋ドメイン)
```

- **melon-core** は I/O を持たず、単体・property テストが容易。
- **felica-rs は melon-auth と melon-terminal のみ**が依存。サーバ本体は rusb を含みません。

## FeliCa 暗号オラクル(melon-auth)

`felica-auth-server`(リモート FeliCa 暗号オラクル、MIT)を取り込んだものです。

- サーバが `keys.jsonl` の DES 鍵を保持(`KeyStore`)。
- 相互認証の各ステップで、端末が中継すべきコマンドフレームを返し、次のリクエストでカード応答を消費(3 ステップの `POST /v1/mutual-authentication`)。
- 認証完了時に `issue_id`(= IDi)と `issue_parameter` を得る。
- セッション(`SessionManager`)は**インメモリ**、TTL でリープ。セッションごとに OS ワーカースレッド + flume チャネルで `felica-rs` の `FelicaStandard` を駆動。

**Melon 固有の拡張**: セッションは認証時の **system_code** と **IDm**、完了後の **検証済み IDi** を保持します。アカウントキーは `(system_code, idm, idi)` の三つ組です。

- `authenticated_account(session_id) -> Option<(system_code, idm, idi)>`
- `consume_spend(session_id) -> Result<(system_code, idm, idi)>`: **一回限り**の支払い能力を消費。1 認証 = 1 金銭操作に束縛し、`session_id` のリプレイを防止。

## デプロイ上の注意

- **v1 はモノリス**: オラクルを `melon-server` に埋め込み、認証セッションと決済を同一プロセスで束縛(tap→決済の結び付きが最も堅い)。
- セッションは**インメモリ**のため、認証と金銭操作は**同一インスタンス**に届く必要があります(単一インスタンス、またはセッションアフィニティ)。残高の真実は共有 PostgreSQL なので台帳のスケール自体は可能。
- 失効スイープは Postgres の **advisory lock** を使うため、複数インスタンスでも 1 回だけ実行されます。
- 将来、鍵隔離(HSM/SAM)やスケールのため、オラクルをサイドカーに分離する余地があります。
