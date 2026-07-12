# HTTP API リファレンス

すべて JSON。金額は整数円、IDm / IDi は 16 桁小文字 hex、System Code は hex(`0x0003`)/10 進(`3`)いずれも可。管理者の口座キーは `(system_code, idm, idi)` の三つ組。

## 認証

| 種別 | 方法 | 対象 |
|---|---|---|
| **加盟店端末(機械)** | `Authorization: Bearer <API キー>` | `/v1/me`、`/v1/transactions`、決済系、相互認証 |
| **ユーザー(人間)** | サインインで得る **HttpOnly Cookie**(`melon_session`) | 管理画面・加盟店ポータルの全操作 |
| **セッション** | 認証済みセッションの `session_id` を本文で参照 | topup / payment / balance |

- 加盟店 API キーは SHA-256 ハッシュで照合。加盟店 `status` が `active` でないと 403。
- **サインオン**: `admin` ロールは `/v1/admin/*` を、`merchant` ロールは加盟店エンドポイントを利用可(加盟店エンドポイントは API キー **または** 加盟店ユーザーのセッションで通る)。

### `POST /v1/auth/login` — サインイン
本文: `{ "email": "...", "password": "..." }` → **200** `{ "user": { id, email, name, role, merchant_id, status, created_at } }`
セッショントークンは**本文に含めず** `Set-Cookie: melon_session=…; HttpOnly; SameSite=Strict` でのみ返します。失敗は 401(メール未登録とパスワード誤りを区別しません)。

### `POST /v1/auth/logout` — サインアウト
サーバ側セッションを削除し Cookie を消去。

### `GET /v1/auth/me` — 現在のユーザー
未サインインは 401。

### `POST /v1/auth/password` — 自分のパスワード変更
本文: `{ "current_password": "...", "new_password": "..." }`(10 文字以上)。成功すると**全セッションが失効**します。

### ユーザー管理
| Method Path | 権限 | 内容 |
|---|---|---|
| `GET/POST /v1/admin/users` | admin | 全ユーザー一覧 / 作成(`role`=`admin`\|`merchant`、merchant は `merchant_id` 必須) |
| `POST /v1/admin/users/{id}/status` | admin | 有効/無効(無効化でセッション失効) |
| `POST /v1/admin/users/{id}/password` | admin | パスワード再設定(セッション失効) |
| `GET/POST /v1/users` | merchant | **自店の**ユーザー一覧 / スタッフ追加(role・merchant はサーバが強制) |
| `POST /v1/users/{id}/status` | merchant | **自店の**ユーザーの有効/無効 |

重複メールは **409 `EMAIL_TAKEN`**。
- 金銭を動かす POST は **`Idempotency-Key` ヘッダ必須**(topup / payment / refund / void)。

## 共通のエラー

`code` は安定した機械可読の識別子で、クライアント(端末キオスク等)はこれを見て**日本語メッセージにローカライズ**します。金額系のエラーは `details` に数値を添えます。

```json
{ "error": { "code": "INSUFFICIENT_FUNDS", "message": "…",
             "details": { "available": 700, "requested": 5000 } } }
```

| HTTP | code の例 | 意味 | details |
|---|---|---|---|
| 400 | `BAD_REQUEST` | 入力不正(範囲外の fee_bps/credit_limit、idi 形式 等) | — |
| 401 | `UNAUTHORIZED` | トークン欠如/不正 | — |
| 403 | `FORBIDDEN` | 加盟店非アクティブ、未認証セッション、支払い能力消費済み | — |
| 404 | `NOT_FOUND` | 加盟店/支払い/口座が存在しない | — |
| 409 | `IDEMPOTENCY_CONFLICT` | 同一キーで異なるパラメータ | — |
| 422 | `INSUFFICIENT_FUNDS` / `CREDIT_LIMIT_EXCEEDED` | 業務ルール違反 | `available`, `requested` |
| 422 | `REFUND_EXCEEDS_PAYMENT` | 返金可能額の超過 | `requested`, `refundable` |

---

## ヘルスチェック / UI

- `GET /healthz` → `{ "status": "ok", "live_sessions": 0 }`
- `GET /admin` → 管理画面 SPA(HTML)
- `GET /merchant` → 加盟店ポータル SPA(HTML)

## 仮名化(加盟店は生の (System Code, IDm, IDi) を見られない)

**加盟店には生のカード識別子を一切返しません。**代わりに **加盟店ごとに異なる仮名 ID(`account_id`, UUID v4)** を返します。

- 同じカードでも**加盟店 A と加盟店 B では別の `account_id`**(結託しても名寄せ不可)。
- 同一加盟店内では**不変**(リピーターを識別できる)。
- `account_id` は**発行元の加盟店にのみ有効**(他店の ID を使うと 404)。
- 生の `(system_code, idm, idi)` を見られるのは**管理者(発行者)のみ**(`/v1/admin/*`)。
- 対応表は `merchant_account_aliases`(追記専用)。

| 面 | 識別子 |
|---|---|
| 加盟店 API(`/v1/balance`, `/v1/transactions`, `/v1/payments/refundable`, 相互認証の完了応答) | `account_id`(仮名)のみ |
| 管理者 API(`/v1/admin/*`) | `system_code` + `idm` + `idi`(生値) |

## 相互認証(加盟店認証)

### `GET /v1/system-codes` — 利用可能なシステムコード
サーバが鍵(`keys.jsonl`)を保持し**認証可能な** FeliCa システムコードの一覧(昇順)。端末はこれを取得し、カードが公開するシステムのうち最初に一致するものを選びます。

```json
{ "system_codes": [3, 65024] }
```

### `POST /v1/mutual-authentication`

3 ステップのリレー。**開始**(session_id なし):

```json
{ "idm": "0101010101010101", "pmm": "0100000000000000",
  "system_code": "0x0003", "areas": ["0x0000"], "services": ["0x0000"] }
```

応答(`command.frame` をカードへ中継):

```json
{ "phase": "mutual_authentication", "step": "auth1",
  "command": { "code": 16, "frame": "10…", "timeout": 0.003 },
  "session_id": "…", "session_created": true }
```

**継続**(`session_id` + `card_response`)を繰り返し、完了時:

```json
{ "phase": "mutual_authentication", "step": "complete",
  "result": { "account_id": "3f2b…-…" }, "session_id": "…" }
```

`result` は**この加盟店の仮名 ID のみ**。生の IDi(`issue_id`)は返しません。以降、この `session_id` に対して 1 回だけ金銭操作が可能(1 認証 = 1 操作)。

## 決済系(加盟店認証 + Idempotency-Key)

### `POST /v1/topups`  — チャージ
本文: `{ "session_id": "…", "amount": 1000 }` → **201**

```json
{ "transaction_id": "…", "bucket_id": "…", "amount": 1000,
  "expires_at": "2027-01-11T…Z", "balance": 1000, "replayed": false }
```
`session_id` から検証済み `(system_code, idm, idi)` を取得。加盟店の与信限度を超える場合は **422 `CREDIT_LIMIT_EXCEEDED`**。

### `POST /v1/payments`  — 支払い
本文: `{ "session_id": "…", "amount": 300 }` → **201**

```json
{ "transaction_id": "…", "amount": 300, "fee": 4, "net": 296,
  "balance": 700, "deductions": [ { "bucket_id": "…", "amount": 300 } ], "replayed": false }
```
`fee` は加盟店手数料、`net`(= amount − fee)は加盟店受取額。残高不足は **422 `INSUFFICIENT_FUNDS`**。

### `POST /v1/refunds`  — 返金
本文: `{ "payment_id": "…", "amount": 100 }`(`amount` 省略で全額)→ **201**

```json
{ "transaction_id": "…", "payment_id": "…", "amount": 100,
  "balance": 800, "restorations": [ { "bucket_id": "…", "amount": 100 } ], "replayed": false }
```
過剰返金は **422 `REFUND_EXCEEDS_PAYMENT`**。自店の支払いのみ操作可(他店は 404)。

### `POST /v1/payments/{payment_id}/void`  — 取消
本文なし → **200**、応答は返金と同形(全額の技術的打消し)。

### `GET /v1/payments/refundable?account_id=&limit=`  — 返金可能な支払い一覧
自店の、指定口座(**仮名 `account_id` で指定**、必須)の、返金余地が残る支払い一覧。端末キオスクの返金フローで使用。他店の `account_id` は **404**。
```json
[ { "id": "…", "account_id": "3f2b…", "amount": 500, "fee": 0,
    "refunded": 0, "refundable": 500, "occurred_at": "…Z" } ]
```

### `POST /v1/balance`  — 認証済みカードの残高
本文: `{ "session_id": "…" }` → **200**

```json
{ "account_id": "3f2b…", "total": 700,
  "buckets": [ { "bucket_id": "…", "remaining": 700, "expires_at": "…Z" } ] }
```

## 加盟店自身(加盟店認証)

### `GET /v1/me`
```json
{ "id": "…", "code": "shop-1", "name": "…", "status": "active",
  "fee_bps": 150, "credit_limit": 50000, "collected": -700, "created_at": "…Z" }
```

### `GET /v1/transactions?kind=&before=&limit=`
呼び出し加盟店の取引に限定。口座は**仮名 `account_id` のみ**(生の `system_code`/`idi` は含まない)。
```json
[ { "id":"…","account_id":"3f2b…","kind":"payment","merchant_id":"…",
    "amount":300,"fee":4,"related_txn_id":null,"occurred_at":"…Z" } ]
```

## 管理: 加盟店(管理者認証)

### `GET /v1/merchants` — 一覧
```json
[ { "id":"…","code":"shop-1","name":"…","status":"active",
    "fee_bps":150,"credit_limit":50000,"collected":-700,"created_at":"…Z" } ]
```

### `POST /v1/merchants` — 作成
本文: `{ "code":"shop-1", "name":"…", "fee_bps": 150, "credit_limit": 50000 }`
（`fee_bps`/`credit_limit` は省略時サーバ既定)→ **201**

```json
{ "merchant_id": "…", "api_key": "…（この時のみ表示）" }
```

### `POST /v1/admin/merchants/{id}/status` — 状態変更
本文: `{ "status": "active" | "suspended" | "closed" }`

### `POST /v1/admin/merchants/{id}/fee` — 手数料率変更
本文: `{ "fee_bps": 250 }`（0〜10000)

### `POST /v1/admin/merchants/{id}/credit-limit` — 与信限度変更
本文: `{ "credit_limit": 100000 }`（≥ 0)

### `POST /v1/admin/merchants/{id}/api-keys` — API キー再発行
本文なし → `{ "merchant_id": "…", "api_key": "…（新規、この時のみ)" }`（既存キーは失効)

### `POST /v1/admin/merchants/{id}/adjust` — 精算残高の調整
本文: `{ "delta": -1000, "reason": "手数料" }` → `{ "id":"…", "delta":-1000, "balance": … }`

## 管理: 口座 / 取引 / 報告(管理者認証)

### `GET /v1/admin/accounts?limit=`
```json
[ { "system_code": 3, "idm": "…", "idi": "…", "status": "active", "balance": 700, "created_at": "…Z" } ]
```

### `GET /v1/admin/accounts/{system_code}/{idm}/{idi}/balance`
残高内訳(管理者は生の `system_code`/`idm`/`idi` 付き)。

### `POST /v1/admin/accounts/{system_code}/{idm}/{idi}/adjust` — 利用者残高の調整
本文: `{ "delta": 500, "reason": "補償" }` → `{ "transaction_id":"…","delta":500,"balance":…,"bucket_id":"…" }`
入金は新 6ヶ月バケット、引落は期限が近い順に消費(残高不足は 422)。

### `GET /v1/admin/transactions?merchant_id=&system_code=&idm=&idi=&kind=&before=&limit=`
`TxnResp` 配列。口座で絞る場合は `system_code`・`idm`・`idi` をすべて指定(三つ組で一意)。

```json
{ "id":"…","system_code":3,"idm":"…","idi":"…","kind":"payment","merchant_id":"…",
  "amount":300,"fee":4,"related_txn_id":null,"occurred_at":"…Z" }
```

### `GET /v1/admin/refundable?merchant_id=&system_code=&idm=&idi=&limit=` — 返金可能な支払い
任意の加盟店/口座で絞り込める返金可能一覧(口座で絞る場合は `system_code`・`idm`・`idi` をすべて指定)。応答は**生の `system_code`/`idm`/`idi` 付き**(管理者のみ)。

### `POST /v1/admin/refunds` / `POST /v1/admin/payments/{id}/void` — 任意の支払いの返金/取消
加盟店の所有者チェックなしで任意の支払いを返金/取消(応答は加盟店版と同形、`Idempotency-Key` 必須)。

### `POST /v1/admin/expiry/sweep` — 失効スイープ実行
→ `{ "ran": true, "expired_buckets": 1, "expired_amount": 1000 }`

### `GET /v1/admin/reports/outstanding-balance?as_of=` — 未使用残高
```json
{ "as_of": "2026-…Z", "total": 201, "account_count": 2,
  "by_expiry_month": [ { "month": "2027-01", "amount": 201 } ] }
```

## 管理: 発行者残高(管理者認証)

発行者の収益残高 = 決済手数料収入 + 消滅済み残高(失効益) + 引き出し・補正。

### `GET /v1/admin/issuer/balance` — 残高と内訳
```json
{ "balance": 1250, "fee_income": 300, "expiry_income": 1000, "adjustments": -50 }
```

### `POST /v1/admin/issuer/adjust` — 引き出し・補正の記帳
本文: `{ "delta": -50000, "reason": "利益引き出し(2026Q2)" }`(delta は非ゼロ、負=引き出し / 正=補正・注入)
→ `{ "id": "…", "delta": -50000, "balance": … }`

### `GET /v1/admin/issuer/adjustments?limit=` — 引き出し・補正の履歴
→ `[ { "id":"…", "amount":-50000, "note":"…", "created_at":"…Z" } ]`(新しい順、`limit` は 1..=500、既定 50)
