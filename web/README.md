# melon-web

Melon の**フロントエンド**(React / Next.js)。発行者向け**管理画面**(`/admin`)と
**加盟店ポータル**(`/merchant`)を提供します。`melon-server` の JSON API とは
完全に分離されており、API の実装には一切手を入れません。

以前は API サーバ(`melon-server`)が `/admin`・`/merchant` の HTML を配信していまし
たが、その役割はこのアプリに移りました。API サーバ側の当該ルートは残置(後方互換)で、
不要になれば削除できます。

## アーキテクチャ — なぜ Next.js か

melon-server のセッションは **HttpOnly・SameSite=Strict の Cookie** です。フロント
を別オリジンに置くと、この Cookie はクロスサイトで送出されず、CORS も必要になります。
そこで [`middleware.ts`](./middleware.ts) が **`/v1/*` と `/healthz` を同一オリジンの
まま API へリバースプロキシ**します。ブラウザから見た通信相手は常にこのフロント自身の
オリジンだけなので、Cookie はファーストパーティのまま流れ、**API 側は無変更**で済みます。

```
ブラウザ ──▶ melon-web (Next.js)
                 │  静的ページ (/admin, /merchant, /)
                 └─ /v1/*・/healthz を MELON_API_ORIGIN へプロキシ ──▶ melon-server (API)
```

プロキシ先は環境変数 **`MELON_API_ORIGIN`**(既定 `http://127.0.0.1:8080`)。
`middleware.ts` が**リクエスト時に**読むため、1 つのビルド/イメージをどの環境でも使えます。

## 開発

```bash
cd web
cp .env.local.example .env.local     # 必要なら MELON_API_ORIGIN を編集
npm install
npm run dev                          # http://localhost:3000
```

別ターミナルで API を起動しておきます(リポジトリルートで):

```bash
DATABASE_URL=postgres://melon:melon@127.0.0.1:5433/melon \
MELON_BIND=127.0.0.1:8080 \
MELON_BOOTSTRAP_ADMIN_EMAIL=admin@example.com \
MELON_BOOTSTRAP_ADMIN_PASSWORD=devpassword123 \
cargo run -p melon-server
```

- 管理画面: http://localhost:3000/admin (発行者アカウントでサインイン)
- 加盟店ポータル: http://localhost:3000/merchant (加盟店ユーザーでサインイン)

役割(admin / merchant)は Cookie セッションの `role` で判定し、画面ごとにガードします。

## 本番ビルド / 実行

```bash
npm run build
MELON_API_ORIGIN=http://127.0.0.1:8080 npm run start   # http://localhost:3000
```

`next.config.mjs` は `output: "standalone"` を指定しているため、Docker では自己完結した
サーバーバンドル(`.next/standalone/server.js`)のみを配置できます。

## Docker

```bash
docker build -t melon-web web/
docker run --rm -p 3000:3000 -e MELON_API_ORIGIN=http://host.docker.internal:8080 melon-web
```

本番(Cloudflare Tunnel)への組み込みは [`deploy/compose.yaml`](../deploy/compose.yaml)
を参照。`cloudflared` の公開ホスト名を **`web:3000`** に向け、`web` が `/v1` を
`server:8080` にプロキシします。端末(melon-terminal)の API 呼び出しも同じ公開ホスト名
経由で `/v1` がプロキシされます。

## 構成

```
app/
  layout.tsx            ルートレイアウト(ToastProvider)
  page.tsx              ランディング(/admin・/merchant への導線)
  admin/                発行者コンソール(7 タブ)
    layout.tsx          PortalShell(role="admin")
    page.tsx            概要 + 失効スイープ
    merchants/          加盟店 CRUD・手数料/与信・APIキー再発行・精算調整
    accounts/           利用者(残高・調整・返金/取消・バケット)
    transactions/       取引フィルタ((sc, idm, idi) 指定)
    report/             未使用残高レポート(資金決済法)
    issuer/             発行者残高・引き出し/補正
    users/              ユーザー管理(発行者/加盟店)
  merchant/             加盟店ポータル(3 タブ: 概要 / 取引 / ユーザー)
components/
  portal.tsx            PortalShell(認証ガード + ヘッダ/ナビ)・useAuth・ログイン
  toast.tsx             ToastProvider / useToast
  ui.tsx                useAsync・Async・Modal・Spinner など
lib/
  api.ts                fetch ラッパ(credentials:'include' + Idempotency-Key)
  types.ts              API レスポンス型
  format.ts             円・hex・日時整形
middleware.ts           /v1・/healthz を API へプロキシ
```

セッショントークンは JavaScript から一切触れません(HttpOnly Cookie をブラウザが自動送出)。
`lib/api.ts` は `credentials: "include"` で同一オリジンへ送るだけです。
