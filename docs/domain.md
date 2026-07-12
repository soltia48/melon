# ドメイン・会計仕様

## アカウント(利用者)

- アカウントは **`(System Code, IDm, IDi)` の複合キー**で識別します(この順序でキーを構成)。IDi(8 バイトの発行 ID)は FeliCa システム内でのみ一意であり、同一カードでもシステムごとに異なる IDi を持つため、IDi 単独ではキーになりません。
- **IDm**(8 バイトの製造 ID)もキーに含めます。IDm はポーリング時に平文で得られる識別子で、カードによってはランダム化されますが、**本デプロイのカードは IDm が固定**であることを前提とします(ランダム化される IDm では毎回別口座に見えてしまうため不可)。
- 口座を持つ全テーブル(`accounts` / `transactions` / `topup_buckets` / `ledger_entries`)が `system_code`(INTEGER, 0〜65535)+ `idm`(BYTEA, 8 バイト)+ `idi`(BYTEA, 8 バイト)を持ち、`(system_code, idm, idi)` の複合 PK / FK で参照します。
- system_code と IDm は認証時に端末が指定・提示したものが採用され、認証完了後の検証済み IDi と組でアカウントになります。
- `accounts.status`: `active` / `frozen` / `closed`。

## 金額

- 通貨は **JPY(円)整数**。補助単位・浮動小数は使いません。
- `Yen(i64)`(符号付き、台帳デルタ用)、`PositiveYen`(> 0 を型で保証、入力ガード)。
- 演算は checked のみ(`From<f64>` や float 乗除は無し)。

## 不変台帳(真実の源)

2 つの粒度で記録します。

- **`transactions`(取引・業務イベント)**: 1 業務イベント = 1 行。種別 `kind`: `top_up` / `payment` / `refund` / `reversal` / `adjustment`。`amount` は正の大きさ。`fee`(支払いの手数料)、`merchant_id`、`idempotency_key`(`UNIQUE(kind, idempotency_key)`)、`related_txn_id`、`note`(調整理由)を持つ。
- **`ledger_entries`(台帳ポスティング)**: 追記専用の**不変**行。1 取引が 1..N 個のポスティングを生成し、同一 `transaction_id` を共有(失効のみ `transaction_id` が NULL)。`amount` は**符号付き**。`bucket_id` で対象バケットを指す。

台帳の種別と符号(DB の `CHECK` で符号↔種別を拘束):

| ledger kind | 符号 | 意味 |
|---|---|---|
| `top_up` | + | チャージによる価値の付与 |
| `payment` | − | 加盟店での支払い |
| `refund` | + | 返金による元バケットへの復元 |
| `expiry` | − | 失効による価値の消滅 |
| `reversal` | ± | 誤記帳の技術的打消し(取消・void) |
| `adjustment` | ± | 管理者による調整 |

**不変性は DB で強制**: `ledger_entries` に対する UPDATE / DELETE はトリガ(`melon_forbid_ledger_mutation`)で拒否。誤り行は消さず、補償行を追記します。

**残高導出**: `remaining(bucket) = SUM(ledger.amount WHERE bucket_id = …)`。`spendable(account, now) = SUM(remaining WHERE 有効かつ expires_at > now)`。`topup_buckets.remaining_amount` は同一トランザクション・行ロック下で維持されるキャッシュ(真実は台帳)。

## 失効バケット(6ヶ月)

- **各チャージ = 1 バケット**(`topup_buckets`)。`original_amount` / `remaining_amount` / `topped_up_at` / `expires_at` / `status`(`active` / `exhausted` / `expired`)。
- **`expires_at = チャージ時刻(JST 壁時計)+ 6 暦月`**。jiff で計算し、**日クランプ**(8/31 + 6ヶ月 → 2/28、3 月に繰り上がらない)。UTC で保存。**チャージ時に materialize**(SQL では再計算しない)。
- 有効なのは `now < expires_at`、`now >= expires_at` で失効。
- タイムゾーンは **Asia/Tokyo(JST, DST なし)**。

### 消費順

支払い・引落は**期限が近い順(soonest-expiry-first)**。tie-break は `topped_up_at` → `id` の決定的順序。利用者の失効額を最小化する消費者保護的な順序です。

### 失効の実現(lazy + eager)

- **Lazy(正しさの権威)**: すべての残高読取・支払いで `expires_at > now()` を絞る。ジョブ実行に依存せず常に正しい。
- **Eager(記帳)**: 定期スイープが期限切れバケットに不変 `expiry` ポスティング(`-remaining`)を追記し `status='expired'` に。Postgres の **advisory lock**(複数インスタンスで 1 回)+ `FOR UPDATE SKIP LOCKED`。冪等。会計・報告を正確化。

## 金銭操作

### チャージ(top_up)

- `(account, merchant_id?, amount, idempotency_key, now, tz)`。新しい 6ヶ月バケットを作成し、取引(`top_up`)+ 台帳(`+amount`)を記録。
- **加盟店による topup は `merchant_id` を記録**し、加盟店の精算残高を減算(後述)。発行者/システム topup は `merchant_id = NULL`。
- **与信限度チェック**(加盟店 topup のみ、後述)。
- 冪等: 同一キーは元の結果を返す。

### 支払い(pay)

1 トランザクション(`READ COMMITTED` + 明示 `FOR UPDATE`):

1. 加盟店の `status`(active)と `fee_bps` を取得。手数料 `fee = floor(amount × fee_bps ÷ 10000)` を計算。
2. 冪等挿入(`ON CONFLICT (kind, idempotency_key) DO NOTHING`)。既存なら台帳から控除を再構築して返す。
3. 対象口座の候補バケットを消費順で `FOR UPDATE` ロック(`expires_at > now`、`remaining > 0`)。
4. 残高不足なら `InsufficientFunds`(422)。
5. 期限が近い順に貪欲に減算し、各バケットに `payment` ポスティング(`-d`)を追記。`CHECK(remaining >= 0)` が最後の砦。

**顧客は満額を支払い**、手数料は加盟店負担(発行者の収益)。負値には決してならず、並行支払いは行ロックで直列化されます。

### 返金 / 取消(refund / void)

- **返金(refund)**: 元の支払いに対し、**元バケットへ復元・有効期限延長なし**。逆消費順で、各ポスティングの元借方額を上限に復元(過剰/二重返金は `RefundExceedsPayment` = 422)。元バケットが既に失効済みなら復元は即時利用不可(失効価値は復活しない)。
- **取消(void)**: 支払いの全額を技術的に打ち消す(`reversal`)。実体は全額返金と同じ機構。
- いずれも冪等。**手数料は返金しません(手数料は非返還)。**

### 残高調整(利用者、管理者)

- 管理者が利用者残高を符号付き `delta` で調整(不変の `adjustment` として記帳、理由 `note` 付き)。
- **入金(credit)**: 新しい 6ヶ月バケットを作成(topup と同様)。
- **引落(debit)**: 期限が近い順に消費。**残高不足は不可**(422、負にならない)。

## 加盟店

### 精算残高(settlement / `collected`)

発行者が加盟店に支払う額(加盟店から見た受取債権)。導出式:

```
精算残高 = Σ(支払 − 手数料)  −  Σ(チャージ)  −  Σ(返金・取消)  +  Σ(加盟店調整)
```

- **支払い**: 加盟店は前払価値を受領 → 発行者が加盟店に債務(+)。手数料は差し引く。
- **チャージ**: 加盟店は顧客から現金を預かる(発行者の現金)→ 加盟店が発行者に債務(−)。
- **返金・取消**: 支払いの巻き戻し(−)。
- **加盟店調整**: 管理者が精算残高を直接増減(`merchant_adjustments`、追記専用)。負値も許容(クローバック等)。

### 決済手数料

- 加盟店ごとに **`fee_bps`**(basis points、1bps = 0.01%、0〜10000)。新規作成時の既定は `MELON_DEFAULT_FEE_BPS`。
- 支払い時に `fee = floor(amount × fee_bps ÷ 10000)` を計算し、**支払いトランザクションに固定記録**(後で料率が変わっても不変)。
- 加盟店の精算は手数料を差し引いた**純額**。顧客残高は満額減算。
- 管理者が料率を変更可能。

### 与信限度(topup 用のマイナス残高)

- 加盟店ごとに **`credit_limit`**(円 ≥ 0)。既定は `MELON_DEFAULT_CREDIT_LIMIT`(既定 **0**)。
- 加盟店は topup を売るほど精算残高がマイナスになるため、**与信限度 = 精算残高が下がってよい下限**。
- **加盟店 topup 後の精算残高が `−credit_limit` を下回る場合のみ拒否**(`CreditLimitExceeded` = 422)。限度内のマイナスは許容。判定中は加盟店行を `FOR UPDATE` でロックし、同時 topup が限度超過しないよう直列化。
- **回転枠モデル**: 与信限度は**現在の精算残高**に対する枠で、支払いを受けると枠が回復する(累計ではなく残高ベース)。UI の「チャージ余力」= `精算残高 + credit_limit`。
- **返金・取消・加盟店調整は与信判定を行わない**(返金は消費者保護のためブロックしない)。したがって**返金により精算残高が `−credit_limit` を下回ることは設計上ありうる**(有界:返金は受領済み支払いの範囲内)。必要なら精算残高の監視/アラートで補完。
- 発行者/システム topup(`merchant_id = NULL`)は対象外。
- **注意**: 既定が 0 のため、与信限度を設定していない加盟店は topup を一切売れません(意図した挙動)。運用では加盟店ごとに設定するか、`MELON_DEFAULT_CREDIT_LIMIT` を設定してください。

### 加盟店の状態・API キー

- `status`: `active` / `suspended` / `closed`。`active` 以外は決済・認証不可。
- **API キー**: SHA-256 ハッシュのみ保存(平文は発行時のみ表示)。**再発行**は既存キーをすべて失効し新規発行。

## 発行者残高(収益)

発行者(運営者)の収益を、既存データからの導出 + 手動記帳で表す**会計上の残高**。現金そのものはチャージを取り扱った加盟店が保持するため、これは「発行者が稼いだ額」であり手元現金ではない。

```
発行者残高 = 決済手数料収入 + 消滅済み残高(失効益) + Σ(発行者調整)
```

| 構成要素 | 導出元 | 意味 |
|---|---|---|
| **決済手数料収入** | `Σ transactions.fee`(kind=`payment`) | 加盟店から徴収した手数料。**非返還**のため、返金済みの決済分も含む |
| **消滅済み残高(失効益)** | `Σ (−ledger_entries.amount)`(kind=`expiry`) | 6ヶ月失効した前払残高の益金(breakage) |
| **発行者調整** | `Σ issuer_adjustments.amount`(符号付き、追記専用) | 利益の**引き出し**(−)、**補正・資本注入**(+) |

- 手数料収入・失効益は既存テーブルからの導出で、二重記帳しない(単一の真実の源)。
- **`issuer_adjustments`**(`0007`、追記専用・不変トリガ)は手動記帳のみを保持。下限なし(将来収益への前渡しとして、引き出しが累計収益を超えてもよい)。
- 会計上の位置づけ: 利用者のチャージ額は最終的に「未使用残高(利用者への負債)」「加盟店精算(加盟店への負債)」「発行者収益(手数料+失効益)」のいずれかへ配分される。発行者残高はこの収益部分。
- API: `GET /v1/admin/issuer/balance`、`POST /v1/admin/issuer/adjust`、`GET /v1/admin/issuer/adjustments`(いずれも管理者認証)。

## 仮名化(加盟店に生のカード識別子を見せない)

加盟店は**生の `(system_code, idm, idi)` を一切見られません**。代わりに **加盟店ごとの仮名 ID `account_id`(UUID v4)** を見ます。

- テーブル **`merchant_account_aliases`**(`0008`、追記専用): `(merchant_id, system_code, idm, idi) → alias`。初回認証時に発行し、以後不変。
- **同一加盟店では不変**(リピーターを識別可能)、**加盟店をまたぐと別 ID**(加盟店同士が結託しても同一人物を名寄せできない)。
- `account_id` は**発行元の加盟店にのみ有効**(他店の ID で照会すると 404)。
- 相互認証の完了応答は `result.issue_id`(生 IDi)ではなく `result.account_id` を返す。
- 対応表を持つのは**発行者のみ**。管理者 API/画面は従来どおり生の `(system_code, idm, idi)` を扱う。
- UUID は **v4**(v7 は生成時刻が漏れるため不採用)。

## 冪等性

- チャージ・支払い・返金・取消は `idempotency_key` を持ち、`UNIQUE(kind, idempotency_key)` で exactly-once を保証。
- リトライは元の結果を返す。異なるパラメータでのキー再利用は `IdempotencyConflict`(409)。

## 不変条件(テストで検証)

各アカウント・任意時刻で:

1. `remaining >= 0`、`remaining <= original`(DB CHECK)
2. `remaining == Σ(ledger on bucket)`
3. `spendable(now) == Σ(remaining WHERE expires_at > now)`
4. 全スイープ後、失効バケットは `Σ(ledger) == 0`
5. 各 payment で `Σ(ポスティング) == -amount`、各借方バケットは `expires_at > occurred_at`

## 未使用残高レポート(資金決済法)

- 基準日(例: 3/31・9/30 JST)時点の未使用残高集計。`GET /v1/admin/reports/outstanding-balance?as_of=`。
- 内容: 総額(`expires_at > as_of` かつ `remaining > 0`)、口座数(`(system_code, idm, idi)` の distinct)、失効月別内訳(JST)。
- 有効期間 6ヶ月以内の前払式支払手段は供託・登録義務等の適用除外を狙う設計(法4条2号、施行令4条2項の趣旨)。**最終判断は要法務レビュー。**
