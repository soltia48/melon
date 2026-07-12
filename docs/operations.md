# 運用・設定

## 環境変数(melon-server)

| 変数 | 既定 | 説明 |
|---|---|---|
| `DATABASE_URL` | (必須) | PostgreSQL 接続文字列 |
| `MELON_BOOTSTRAP_ADMIN_EMAIL` | — | 初回起動時に作成する管理者のメール(管理者が 1 人も居ない時だけ作成) |
| `MELON_BOOTSTRAP_ADMIN_PASSWORD` | — | 同パスワード(10 文字以上) |
| `MELON_USER_SESSION_TTL` | `43200` | サインインセッションの寿命(秒、既定 12 時間) |
| `MELON_COOKIE_SECURE` | (自動) | セッション Cookie に `Secure` を付ける。既定は**バインド先が loopback 以外なら true**(TLS 前提)。ローカル HTTP 開発では自動的に false |
| `MELON_KEYS` | `keys.jsonl` | FeliCa 鍵ファイルのパス(秘匿) |
| `MELON_BIND` | `127.0.0.1:8080` | バインドアドレス |
| `MELON_SWEEP_INTERVAL_SECS` | `3600` | 失効スイープの実行間隔(秒) |
| `MELON_DEFAULT_FEE_BPS` | `0` | 新規加盟店の既定手数料率(bps) |
| `MELON_DEFAULT_CREDIT_LIMIT` | `0` | 新規加盟店の既定与信限度(円) |
| `FELICA_SESSION_TTL` | `300` | 認証セッションのアイドル TTL(秒) |
| `FELICA_MAX_SESSIONS` | `1024` | 同時セッション上限 |
| `RUST_LOG` | `info` | ログレベル(tracing) |

> ⚠️ `MELON_DEFAULT_CREDIT_LIMIT` が 0 の場合、与信限度未設定の加盟店は topup を売れません(最初の topup で精算残高がマイナスになり拒否)。加盟店ごとに与信限度を設定するか、この既定値を設定してください。

## データベース

- **PostgreSQL**(sqlx)。マイグレーションは**サーバ起動時に自動適用**(`sqlx::migrate!`)。
- **開発用**は専用コンテナ `melon-postgres`(ホスト `127.0.0.1:5433`、user/pass/db = melon)。リポジトリ直下 `compose.yaml`(compose プロジェクト **`melon`**)。`#[sqlx::test]` がテストごとに一時 DB を作るため、テスト実行にも必要です:

```bash
docker compose up -d db
export DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon
```

> **compose ファイルは 2 つあり、用途が違います**
>
> | ファイル | プロジェクト名 | 用途 |
> |---|---|---|
> | `compose.yaml`(直下) | `melon` | **開発専用**。Postgres 1 台のみ(ホスト :5433、認証情報は自明) |
> | `deploy/compose.yaml` | `melon-prod` | **本番**。cloudflared + melon-server + Postgres。**公開ポートはゼロ**(Cloudflare Tunnel 経由) |
>
> プロジェクト名を分けてあるので、`deploy/` での `docker compose down` が開発 DB を巻き込むことはありません。

### マイグレーション(`crates/melon-db/migrations/`)

| ファイル | 内容 |
|---|---|
| `0001_init` | accounts / merchants / transactions / topup_buckets / ledger_entries、追記専用トリガ |
| `0002_merchant_api_keys` | 加盟店 API キー(ハッシュ保存) |
| `0003_transaction_note` | 調整理由 `transactions.note` |
| `0004_merchant_adjustments` | 加盟店精算残高の調整(追記専用) |
| `0005_payment_fees` | `merchants.fee_bps` / `transactions.fee` |
| `0006_merchant_credit_limit` | `merchants.credit_limit` |
| `0007_issuer_ledger` | 発行者の引き出し・補正(`issuer_adjustments`、追記専用) |
| `0008_merchant_account_aliases` | 加盟店ごとの仮名 ID(`merchant_account_aliases`、追記専用) |
| `0009_users` | ユーザーアカウント(`users`)+ サーバ側セッション(`user_sessions`) |

- 開発中にスキーマ基盤を変更してリセットが必要な場合:
  `docker exec melon-postgres psql -U melon -d melon -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"` → サーバ再起動で全マイグレーション再適用。

## 鍵ファイル(`keys.jsonl`)

1 行 1 JSON(felica-rs の `keys.jsonl` 形式)。**秘匿情報。リポジトリにコミットしない**(`.gitignore` 済み)。

```json
{"system_code":"0003","node":"FFFF","algo":"DES","version":"0003","idm":null,"key":"00112233445566FF"}
```

- `system_code` / `node`: hex。node `FFFF` は**システム鍵**、その他はエリア/サービス鍵。
- `algo`: `DES`(8 バイト鍵)。`AES` レコードは無視(本オラクルは DES で認証)。
- `idm`: `null`(システム共通鍵)または 8 バイト hex(カード個別鍵。一致時に優先)。
- `key`: 鍵(hex)。

> **⚠️ システムコードの記述順 = 優先順位**
> `GET /v1/system-codes` はこのファイルでの **`system_code` の初出順**をそのまま返します。端末は**この順序を優先**して、カードが持つ最初のシステムを選びます(§端末)。マルチシステムカードでどのシステムを使うかは、**`keys.jsonl` の並び順で制御**してください(例: 0xFE00 を先に書けば 0xFE00 が優先される)。DES 鍵の無いシステム(AES のみ等)は一覧に含まれません。

## サーバの起動

```bash
export DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon
export MELON_KEYS=keys.jsonl
# 初回のみ: 最初の管理者ユーザーを作成(管理者が居ない時だけ作成される)
export MELON_BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export MELON_BOOTSTRAP_ADMIN_PASSWORD='<10 文字以上>'
# 任意: MELON_DEFAULT_FEE_BPS / MELON_DEFAULT_CREDIT_LIMIT など
cargo run -p melon-server        # 開発
# 本番向け(rusb フリー成果物): cargo build -p melon-server --release
```

## サインオン(ユーザーアカウント)

**人間はユーザーアカウント(メール + パスワード)でサインイン**します。端末(機械)は従来どおり**加盟店 API キー**を使い、人間の資格情報とは分離されています。

- **パスワード**: Argon2id(メモリハード、パスワードごとのソルト、PHC 文字列で保存)。10 文字以上。
- **セッション**: サーバ側(`user_sessions`)。Cookie は **HttpOnly / SameSite=Strict**(+ TLS 下では `Secure`)なので **JS からは読めず、XSS で盗めません**。保存されるのはトークンの SHA-256 のみ。行を消せば**即時失効**。
- **ロール**: `admin`(発行者スタッフ)/ `merchant`(加盟店スタッフ、1 加盟店に紐づく)。
- **ユーザー管理**: 管理者は全ユーザーを作成/無効化/パスワード再設定。**加盟店は自店のスタッフのみ**追加/無効化でき、admin や他店ユーザーは作れません(サーバ側で強制)。
- 無効化・パスワード変更は、そのユーザーの**全セッションを失効**させます。

**初回セットアップ**(管理者が 1 人も居ない場合のみ作成、冪等):

```bash
export MELON_BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export MELON_BOOTSTRAP_ADMIN_PASSWORD='<10 文字以上>'
cargo run -p melon-server     # 起動時に管理者ユーザーを作成
```

> ⚠️ 旧 `MELON_ADMIN_TOKEN`(共有トークン)は**廃止**しました。管理者操作はユーザーサインインが必要です。

## Web UI(管理画面・加盟店ポータル)

フロントエンドは**別アプリ** [`web/`](../web/)(React/Next.js)です。API サーバは HTML を
配信せず、`web` が `/v1` を同一オリジンで API へプロキシします(HttpOnly Cookie を維持)。
起動・デプロイ手順は [`web/README.md`](../web/README.md)。

- **管理画面** `http://<web-host>/admin` — 発行者アカウントでサインイン。
  - 概要(未使用残高・発行者残高・口座数・加盟店数・セッション、失効スイープ実行)
  - 加盟店(作成〔手数料・与信限度〕、一覧、詳細で精算残高調整・状態変更・手数料/与信限度変更・API キー再発行)
  - 利用者(残高一覧、(system_code, idm, idi) で照会、入金/引落調整、**返金可能な支払いの返金/取消**)
  - 取引(フィルタ)、未使用残高レポート
  - 発行者(収益残高=決済手数料収入+失効益+調整の内訳、引き出し・補正の記帳と履歴)
  - ユーザー(発行者/加盟店ユーザーの作成・無効化・パスワード再設定)
- **加盟店ポータル** `http://<web-host>/merchant` — 加盟店アカウントでサインイン。
  - 概要(精算残高・手数料率・与信限度・チャージ可能額)、自店取引、自店支払いの返金/取消
  - ユーザー(自店スタッフの追加/無効化、自分のパスワード変更)

## 端末(melon-terminal)

PaSoRi(Sony RC-S380 等)を駆動しフレームを中継する加盟店端末。**要 USB リーダ + 実カード**。2 モードあり、リーダ/相互認証中継/サーバ呼び出しの共通ロジックはライブラリ(`melon_terminal`)を共有します。**モードは引数で決まります**:操作(`--op` または `--amount`)を指定すると CLI 一発実行、指定しなければ **Web UI キオスク(デフォルト)**。

> **ビルド済みバイナリ**: `v*` タグを push すると GitHub Actions([.github/workflows/release.yml](../.github/workflows/release.yml))が Linux(x86_64 / arm64)・Windows(x86_64)・macOS(Apple Silicon)向けにビルドし、その Release にアーカイブを添付します。libusb はバンドル(vendored)するため実行環境に libusb のインストールは不要です。CI がプライベート依存 `felica-rs` を取得するため、リポジトリに **`FELICA_RS_TOKEN`** シークレット(`soltia48/felica-rs` を読める PAT)が必要です。

**① Web UI キオスク(デフォルト)** — 操作フラグなしで起動すると、リーダを占有する常駐プロセスがローカル Web UI とローカル JSON API を**同一 `http://localhost` オリジン**で提供(CORS/mixed-content 回避)。**起動時にデフォルトブラウザで UI を自動的に開きます**(`--no-open` で抑止)。タッチ操作で決済・チャージ・残高照会:

```bash
# 引数なしで起動 → キオスク(既定バインド 127.0.0.1:8899)。
# 起動するとブラウザで http://127.0.0.1:8899/ が自動で開きます。
# API キーは画面の「⚙ 設定」から入力でき、以後保存されて再起動時に自動読込されます。
cargo run -p melon-terminal

# API キーを事前に渡す/バインドを変える/ブラウザを開かない場合(操作フラグは付けない):
cargo run -p melon-terminal -- --api-key <加盟店 API キー> --bind 127.0.0.1:8899 --no-open
```

**② 一発実行(CLI)** — `--op` か `--amount` を指定するとこちら。カード待機 → 認証 → 1 操作 → 結果表示 → 終了。スクリプト/実機立ち上げ向け(この場合 `--api-key` は必須):

```bash
cargo run -p melon-terminal -- \
  --server http://127.0.0.1:8080 \
  --api-key <加盟店 API キー> \
  --op pay --amount 500
# `--amount 500` だけでも可(--op は既定で pay)
```

- **ブラウザ自動起動**: キオスク起動時、`http://<bind>/`(ワイルドカードbindは `127.0.0.1` に変換)をデフォルトブラウザで開きます。ヘッドレス/リモートでは `--no-open` で抑止(開けなくても警告のみで動作は継続)。
- **API キーを画面から設定・保存**: キオスク起動時は `--api-key`(`MELON_API_KEY`)は任意です。未設定で起動すると、キオスク画面が「⚙ 設定」を開いて API キーの入力を求めます(ヘッダーの「⚙ 設定」からいつでも変更可)。入力キーはサーバの `/v1/system-codes` 取得で検証してから採用します。**検証に成功したキーは OS の設定ディレクトリに保存**され(`~/.config/melon-terminal/credentials.json` など、**パーミッション 0600**)、次回以降 `--api-key` なしで起動しても自動読込されます。`--api-key` 指定時はそちらを優先。`GET /config` は設定済みか否か(真偽)だけを返し、**キーそのものは画面へ返しません**。
- **加盟店 API キー・サーバ URL はプロセス内に留め、ブラウザには渡さない**。UI はローカル `/config`・`/op/pay|topup`・`/op/balance`・`/op/refund*`・`/status`・`/cancel`・`/reset`・`/me` のみを叩く。
- **利用者の表示**: 画面に出るのは**仮名 ID(`account_id`)のみ**。端末は生の IDi をサーバから受け取りません(加盟店ごとに異なる仮名 ID)。
- **加盟店情報**: ヘッダー右上の「ⓘ 加盟店情報」から自店情報(精算残高・チャージ可能額・手数料率・与信限度・コード/名称/状態/登録日時)を閲覧。プロセスが `/v1/me` を代理取得(`/me`)。操作(支払い/チャージ/残高/返金)と情報を分離し、上部タブは操作 4 種に限定。
- リーダは単一リソースのため、**リーダ専有スレッドが 1 操作ずつ**実行(HTTP ループはジョブ投入と状態返却のみでカード待ちにブロックしない)。同時操作は 409。
- 画面フロー:金額入力 →(支払う/チャージ/残高)→「カードをかざしてください」→ 認証中 → 完了/エラー。待機中は**キャンセル**可能。失敗時は再読み取り・再送せずエラー表示(一発実行と同じ一回試行の原則)。
- **返金**:「返金」タブ → カード提示 → その利用者の**返金可能な支払い一覧**から選択 → 金額(既定=全額)→ 実行。照会(要リーダ)と実行(リーダ不要)の 2 段構成。
- 金額は**オンスクリーンのテンキー**と**物理キーボード**の両方で入力可(数字/Backspace/Enter=実行/Escape=クリア・キャンセル)。
- **効果音**:決済・チャージ・残高確認・エラーで異なる合成音(Web Audio、外部ファイル不要・オフライン可)。ブラウザの仕様上、最初の操作(タップ/キー)で音声が有効化されます。

| オプション | 既定 | 説明 |
|---|---|---|
| `--server` (`MELON_SERVER`) | `https://melon.unknowntech.jp` | サーバ URL |
| `--api-key` (`MELON_API_KEY`) | — | 加盟店 API キー。**一発実行は必須**、キオスクは任意(画面から設定可) |
| `--op` | — | `pay` / `topup` / `balance`。**指定すると一発実行モード**(既定 `pay`) |
| `--amount` | — | 金額(円)。**指定すると一発実行モード**。`pay`/`topup` は必須、`balance` は不要 |
| `--bind` | `127.0.0.1:8899` | キオスクのバインドアドレス(操作フラグ未指定=キオスク時に使用) |
| `--no-open` | (開く) | キオスク起動時にデフォルトブラウザを開かない(ヘッドレス/リモート向け) |
| `--poll-interval-ms` | `500` | カード待機中のポーリング間隔 |
| `--timeout-secs` | `0` | カード待ちの上限(0 = 無期限、一発実行のみ) |
| `-v` / `-vv` (`RUST_LOG`) | (info) | コンソールログの詳細度(下記) |

**デバッグログ**(**stderr** に出力。**stdout** は操作結果のみなのでパイプ可能):

| レベル | 指定 | 内容 |
|---|---|---|
| **info**(既定) | — | 動作の流れ:リーダ検出、サーバから取得したシステムコード、カード待機/検出、カードのシステムコード一覧と**選択したシステム**、相互認証の完了(**仮名 `account_id`**)、実行した操作 |
| **debug** | `-v` | + サーバとの HTTP(メソッド/パス/ステータス)、**Request System Code の送受信フレーム**、相互認証の**中継フレーム(各ステップ)**、選択システムでの再ポーリング結果(IDm/PMm) |
| **trace** | `-vv` | + リクエスト/レスポンスの**生ボディ**、**毎回のポーリング**(カード無し含む) |

`RUST_LOG` を設定するとそちらが優先(例: `RUST_LOG=melon_terminal=debug`、`RUST_LOG=warn`)。**加盟店 API キーはログに出力しません**。キオスク(デフォルト)モードでは、UI からのローカルリクエスト・ジョブの開始/完了・失敗時のエラーコードもログに出ます。

**カード検出とシステム選択のフロー**:

1. **サーバから利用可能なシステムコードを取得**(`GET /v1/system-codes` = 鍵を持つシステム。**優先順位付き**)。起動時に 1 回。
2. **ワイルドカード(0xFFFF)で Polling** — カードが present になるまで待機。
3. 応答したシステムの IDm に対し **Request System Code** を発行し、カードが持つシステムコード一覧を取得。
4. **サーバの一覧を順に走査**し、カードが持つ最初のシステムコードを選択(**サーバ側の順序が優先** — マルチシステムカードの内部配置に左右されず、どのシステムで取引するかをサーバが決める)。
5. **選択したシステムコードで再ポーリング** — FeliCa は**システムごとに IDm が異なる**ため、ワイルドカードで得た IDm は再利用できない(再利用すると Authentication1 が失敗する)。ここで得た IDm/PMm で相互認証 → 取引。

**挙動**:

- **カード無し**は待機(ポーリングのタイムアウトは正常)。`--timeout-secs` で打ち切り可。
- カードを検出したら **1 回だけ試行**。サーバの鍵に一致するシステムがカードに無い、認証失敗、サーバエラー、残高不足等、**何らかの失敗で異常終了**(再読み取り・再送はしない)。キオスクでは「対応していないカードです」等を日本語表示。
- エリア/サービスは **0x0000 固定**。
- USB アクセス権限が必要(Linux では対象ユーザーが `plugdev` 等に所属、または udev ルール)。

## 本番デプロイ(`deploy/`)

`deploy/compose.yaml` + ルートの `Dockerfile`。**Cloudflare Tunnel(cloudflared・ダッシュボード管理のトークン方式)で公開**するため、**リバースプロキシは無し**・**インバウンドポートも一切開けません**。

```
  ブラウザ・端末 ──HTTPS──▶ Cloudflare edge
                                 │ (アウトバウンドのみのトンネル)
                            cloudflared ──▶ server ──▶ db
```

**1. Cloudflare 側(Zero Trust ダッシュボード)**

1. **Networks → Tunnels → Create a tunnel → Cloudflared** でトンネルを作成し、**トークンをコピー**
2. **Public hostname** を追加:
   - Subdomain/Domain: `melon.example.com`
   - Service: **HTTP** → **`server:8080`** ← **compose のサービス名**。`localhost:8080` ではありません(よくある誤り)

**2. サーバ側**

```bash
cd deploy
cp env.example .env && chmod 600 .env    # CLOUDFLARE_TUNNEL_TOKEN を貼る
./init-secrets.sh                        # DB/管理者パスワードを生成
cp /path/to/keys.jsonl secrets/          # FeliCa 鍵
docker compose up -d
```

### 機密の扱い
DB・管理者パスワード・FeliCa 鍵は**ファイル**で渡し、環境変数には置きません(`docker inspect` や `/proc/<pid>/environ` から読めるため)。サーバは `<VAR>_FILE` に対応しています。

| 場所 | 用途 |
|---|---|
| `secrets/keys.jsonl` | **FeliCa DES 鍵。最重要**。これがあれば誰でもカードを認証できる。イメージに焼かない・コミットしない |
| `secrets/database_url` | `postgres://melon:<pass>@db:5432/melon` |
| `secrets/db_password` | Postgres の `POSTGRES_PASSWORD_FILE` |
| `secrets/bootstrap_admin_password` | 初回管理者のパスワード |
| `.env` の `CLOUDFLARE_TUNNEL_TOKEN` | **唯一の例外**(下記) |

`deploy/secrets/` はディレクトリを **700**、`deploy/.env` は **600**。すべて `.gitignore` 済み。

> **⚠️ トンネルトークンだけは環境変数になります**
> `cloudflare/cloudflared` イメージは **distroless(シェル無し)** で `*_FILE` にも非対応のため、トークンをファイルから読めません。結果として **`docker inspect` できる者には見えます**。`.env` を 600 にし、docker ソケットへのアクセスを制限してください。漏洩時はダッシュボードから**ローテート**すれば無効化できます(このトークンで可能なのはトンネルの実行のみ)。

### 譲れない制約

- **⚠️ 単一インスタンス**。FeliCa 相互認証セッションは**サーバのメモリ上**にあるため、カードのタップと後続の金銭操作は**同一プロセス**に届く必要があります。`replicas: 1` を上げるとセッションアフィニティ無しでは**決済が全て失敗**します(台帳は PostgreSQL なのでスケール自体は将来可能)。
- **⚠️ Cookie は `Secure` のまま**。TLS は Cloudflare のエッジで終端されるため、cloudflared → server 間が平文 HTTP でも**ブラウザから見れば HTTPS** です。「コンテナが HTTP だから」と `MELON_COOKIE_SECURE` を false にしてはいけません。
- **⚠️ セキュリティヘッダはサーバが付与**。リバースプロキシが無いので、HSTS / CSP / X-Frame-Options / X-Content-Type-Options / Referrer-Policy は **melon-server 自身**が全レスポンスに付けます(HSTS は `MELON_COOKIE_SECURE=true` のときだけ — 平文 HTTP の開発ホストに HSTS を焼き付けないため)。
- **⚠️ felica-rs はプライベート git 依存。ビルド時に SSH で取得します**(vendor しません)。`docker compose build` が **SSH エージェントを転送**(BuildKit の `--mount=type=ssh`)するので、ビルドホストで **ssh-agent にリポジトリ権限のある鍵を登録**しておく必要があります:
  ```bash
  eval "$(ssh-agent -s)" && ssh-add ~/.ssh/id_ed25519   # 鍵を登録
  cd deploy && docker compose build                      # SSH で felica-rs を取得
  ```
  ローカルの `cargo build` も同様に SSH で取得します(`.cargo/config.toml` の `git-fetch-with-cli = true`)。rev は `Cargo.toml` で固定(usb feature コミット)。Docker コンテキストは `deploy/` の親=リポジトリ直下(`context: ..`)。
- サーバは **`-p melon-server` のみ**をビルド(ワークスペース全体だと端末の `usb` feature が統合され rusb がリンクされる)。

### 運用メモ

- **公開ポートはゼロ**。cloudflared が Cloudflare へ**アウトバウンド接続**するだけなので、ファイアウォールに穴を開ける必要がありません。DB もサーバもホストからは見えません。
- **経路(ingress)は Cloudflare ダッシュボード側**にあります。誰かが Public hostname の向き先を変更できてしまうため、**Cloudflare アカウントの権限管理も本番の攻撃面**です。
- **マイグレーションは起動時に自動適用**(`sqlx::migrate!`)。デプロイ順序の考慮は不要。
- **DB バックアップは別途必須**(不変台帳のため復旧不能な損失になる):
  `docker compose exec -T db pg_dump -U melon melon | gzip > melon-$(date +%F).sql.gz`
- コンテナは **非 root・`read_only`・`cap_drop: ALL`・`no-new-privileges`**。ヘルスチェックは `/healthz`。
- 任意: **Cloudflare Access** で `/admin` を追加保護できます(サインオンの手前に IdP を置く)。

## ビルド上の注意

- **サーバは `-p melon-server` で個別ビルド**すれば rusb を含みません。ワークスペース全体の `cargo build` は端末の `usb` feature が統合され、felica-rs(rusb)がリンクされます。
- 端末のビルド/実行には libusb が必要。

## テスト

```bash
export DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon
cargo test --workspace          # 全テスト(melon-db/server の結合は #[sqlx::test] で毎回一時 DB を作成)
cargo clippy --workspace --all-targets
```

- **melon-core**: 金額/失効/消費順/手数料の単体・property テスト。
- **melon-db**: 実 Postgres での結合(二重支払い・冪等・失効・返金/取消・与信限度・手数料・精算 等)。
- **melon-auth**: インメモリカードエミュレータでの相互認証。
- **melon-server**: E2E(加盟店作成 → 相互認証 → チャージ → 支払い → 残高、を HTTP 経由で)。

## 未対応・今後の候補

- 加盟店認証は API キー(Bearer)。HMAC 署名(ts/nonce リプレイ防止)は未実装。
- 手数料は非返還(返金時の比例返還なし)。
- オフライン/後方キャプチャ、加盟店ユーザーアカウント(個別ログイン)等は未対応。
