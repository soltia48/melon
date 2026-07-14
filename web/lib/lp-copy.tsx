import type { ReactNode } from "react";

/**
 * Every string on the landing page, in each language it is published in.
 *
 * The markup lives in `components/landing.tsx` and takes one of these; keeping
 * the copy out of the component is what lets / and /en be the same page in two
 * languages instead of two pages that drift apart.
 *
 * The legal documents are Japanese only — they are the binding text — so the
 * English footer links to them with the language spelled out rather than
 * pretending an English version exists.
 */
export type Locale = "ja" | "en";

interface Item {
  t: string;
  b: ReactNode;
}

export interface LpCopy {
  locale: Locale;
  /** Where the language toggle in the nav points, and what it says. */
  other: { href: string; label: string; hreflang: string };
  nav: {
    features: string;
    how: string;
    terminals: string;
    contact: string;
    portal: string;
  };
  hero: {
    eyebrow: string;
    h1: ReactNode;
    lead: ReactNode;
    noApp: ReactNode;
    noCard: ReactNode;
    portal: string;
    contact: string;
    microcopy: string;
    tapline: string;
    tapsub: string;
    balance: string;
    expiry: string;
  };
  values: { tap: Item; noCard: Item; merchant: Item; note: string };
  features: {
    eyebrow: string;
    h2: string;
    lead: string;
    identity: Item;
    ledger: Item;
    expiry: Item;
    ops: Item;
  };
  how: {
    eyebrow: string;
    h2: string;
    lead: string;
    tap: Item;
    auth: Item;
    settle: Item;
  };
  terminals: {
    eyebrow: string;
    h2: string;
    lead: string;
    download: string;
    android: Item & { pill: string };
    desktop: Item & { pill: string };
    api: Item & { pill: string };
  };
  cta: { h2: string; lead: string; contact: string; portal: string };
  footer: {
    about: string;
    product: string;
    merchantHead: string;
    resources: string;
    legal: string;
    portal: string;
    guide: string;
    contact: string;
    dlDesktop: string;
    dlAndroid: string;
    terms: string;
    merchantTerms: string;
    note: string;
  };
}

export const LP_JA: LpCopy = {
  locale: "ja",
  other: { href: "/en", label: "English", hreflang: "en" },
  nav: {
    features: "特徴",
    how: "仕組み",
    terminals: "端末",
    contact: "お問い合わせ",
    portal: "加盟店ポータル",
  },
  hero: {
    eyebrow: "オンライン前払式支払手段プラットフォーム",
    // Japanese wraps anywhere, so a narrow viewport would cut a phrase in half
    // ("支払い / も、"). Each 文節 is its own box, so the only place a line can
    // break is between them — see `.lp .hero h1 .ph` in globals.css.
    h1: (
      <>
        <span className="ph">かざすだけ。</span>
        <br />
        <span className="ph">支払いも、</span>
        <span className="ph">チャージも、</span>
        <span className="ph">
          <span className="hl">一瞬</span>で。
        </span>
      </>
    ),
    lead: (
      <>
        利用者が新たに用意するものは、ありません。お手持ちの FeliCa
        が、そのまま支払い手段になります。残高はサーバ側の台帳で安全に管理し、相互認証でカードの真正性を検証します。
      </>
    ),
    noApp: (
      <>
        専用アプリのインストール<b>不要</b>
      </>
    ),
    noCard: (
      <>
        専用カードの発行<b>不要</b>
      </>
    ),
    portal: "加盟店ポータルへ",
    contact: "導入のお問い合わせ",
    microcopy: "加盟店 API キーだけで、Android・デスクトップ端末・API から。",
    tapline: "カードをかざしてください",
    tapsub: "FeliCa Standard · 相互認証で検証済み",
    balance: "利用可能残高",
    expiry: "有効期限",
  },
  values: {
    tap: {
      t: "かざすだけの操作",
      b: "支払いもチャージも、カードをかざすだけ。迷う操作はありません。",
    },
    noCard: {
      t: "アプリも専用カードも不要",
      b: (
        <>
          お手持ちの FeliCa が、そのまま Melon
          に。新しいカードの発行も待ち時間もありません。
        </>
      ),
    },
    merchant: {
      t: "加盟店はキー1つで導入",
      b: "API キーを入れるだけ。Android でもデスクトップでもすぐに。",
    },
    note: "※ ご利用には、当社が対応するものとして指定した FeliCa が必要です。すべての FeliCa でご利用いただけるものではありません。",
  },
  features: {
    eyebrow: "なぜ Melon か",
    h2: "タップの正しさを、仕組みで担保する。",
    lead: "信頼の起点はカードの主張ではなく、サーバ側のオンライン相互認証。その上に、消せない台帳と規制に配慮した失効を重ねています。",
    identity: {
      t: "サーバ検証済みの本人性",
      b: (
        <>
          オンライン相互認証で得た IDi
          をアカウントキーに。加盟店は暗号鍵を持たず、残高もカードに書かないため、なりすましも改ざんもできません。
        </>
      ),
    },
    ledger: {
      t: "追記専用の不変台帳",
      b: "すべての入出金を消せない台帳に記録。冪等キーで二重支払いを防ぎ、返金は元のチャージへ正しく戻します。残高はいつでも台帳から再構築可能。",
    },
    expiry: {
      t: "6 か月失効(資金決済法に配慮)",
      b: (
        <>
          チャージごとに JST で 6 か月失効。発行日から 6
          か月以内の適用除外を、厳密・監査可能・不変に実装しています。
        </>
      ),
    },
    ops: {
      t: "取引・返金・精算を一元管理",
      b: "残高・取引・返金・精算・スタッフ管理まで。加盟店は専用ポータルから、日々の運用に必要な操作だけを役割ごとに行えます。",
    },
  },
  how: {
    eyebrow: "仕組み",
    h2: "タップが、決済になるまで。",
    lead: "端末はカードとサーバのフレームを中継するだけ。暗号鍵はサーバが握り、決済はサーバの台帳で確定します。",
    tap: {
      t: "カードをかざす",
      b: (
        <>
          加盟店端末が FeliCa をポーリングし、IDm / PMm
          を取得。端末は鍵を持ちません。
        </>
      ),
    },
    auth: {
      t: "サーバで相互認証",
      b: (
        <>
          端末はフレームを中継するだけ。サーバが鍵で認証し、検証済み IDi
          と認証済みセッションを得ます。
        </>
      ),
    },
    settle: {
      t: "残高で決済",
      b: "そのセッションで支払い・チャージ・残高照会・返金。台帳に記帳して完了します。",
    },
  },
  terminals: {
    eyebrow: "対応端末",
    h2: "どの端末からでも、同じ台帳へ。",
    lead: "加盟店 API キーで認証。金銭操作は冪等キーで二重処理を防ぎます。",
    download: "ダウンロード",
    android: {
      t: "Android アプリ",
      b: "NFC 対応端末を、かざすだけの POS 端末に。テンキー入力と確認ボタンで誤タップも防止。",
      pill: "NFC-F / FeliCa",
    },
    desktop: {
      t: "デスクトップ端末",
      b: (
        <>
          PaSoRi を接続した常設レジ向け。ブラウザの Web UI
          キオスクとして起動します。
        </>
      ),
      pill: "Win / macOS / Linux",
    },
    api: {
      t: "REST API",
      b: (
        <>
          相互認証・決済・返金・残高照会を JSON
          で。独自端末や基幹システムへ直接統合。
        </>
      ),
      pill: "/v1",
    },
  },
  cta: {
    h2: "導入をご検討ですか?",
    lead: "Melon の導入・お見積りは、お問い合わせページからご相談ください。すでに加盟店の方は、ポータルからサインインできます。",
    contact: "お問い合わせ",
    portal: "加盟店ポータル",
  },
  footer: {
    about: "FeliCa IDi ベースのオンライン前払式支払手段プラットフォーム。",
    product: "プロダクト",
    merchantHead: "加盟店",
    resources: "リソース",
    legal: "規約",
    portal: "加盟店ポータル",
    guide: "オペレーションマニュアル",
    contact: "お問い合わせ",
    dlDesktop: "デスクトップ版ダウンロード",
    dlAndroid: "Android 版ダウンロード",
    terms: "利用規約",
    merchantTerms: "加盟店規約",
    note: "Melon は前払式支払手段(第三者型)の発行・管理基盤です。",
  },
};

export const LP_EN: LpCopy = {
  locale: "en",
  other: { href: "/", label: "日本語", hreflang: "ja" },
  nav: {
    features: "Why Melon",
    how: "How it works",
    terminals: "Terminals",
    contact: "Contact",
    portal: "Merchant portal",
  },
  hero: {
    eyebrow: "Online prepaid payment platform",
    h1: (
      <>
        Just tap.
        <br />
        Pay or top up in an <span className="hl">instant</span>.
      </>
    ),
    lead: (
      <>
        There is nothing for your customers to get hold of. The FeliCa they
        already carry becomes the payment method. Balances are held in a
        server-side ledger, and mutual authentication proves the card is
        genuine.
      </>
    ),
    noApp: (
      <>
        <b>No</b> app to install
      </>
    ),
    noCard: (
      <>
        <b>No</b> card to issue
      </>
    ),
    portal: "Merchant portal",
    contact: "Talk to us",
    microcopy:
      "One merchant API key — from Android, a desktop terminal, or the API.",
    tapline: "Please tap your card",
    tapsub: "FeliCa Standard · verified by mutual authentication",
    balance: "Available balance",
    expiry: "Expires",
  },
  values: {
    tap: {
      t: "One tap, nothing else",
      b: "Paying and topping up are the same gesture: hold the card to the reader. There is nothing to work out.",
    },
    noCard: {
      t: "No app, no dedicated card",
      b: "The FeliCa your customers already carry becomes their Melon. No card to issue, no waiting.",
    },
    merchant: {
      t: "Merchants start with one key",
      b: "Paste in an API key and you are live — on Android or on the desktop.",
    },
    note: "* Melon works only with FeliCa that we have designated as supported. Not every FeliCa can be used.",
  },
  features: {
    eyebrow: "Why Melon",
    h2: "A tap you can trust, by construction.",
    lead: "Trust starts with online mutual authentication on the server, not with what a card claims. On top of that sit an immutable ledger and an expiry scheme built for the regulation.",
    identity: {
      t: "Identity the server verified",
      b: (
        <>
          The account key is the IDi obtained through online mutual
          authentication. Merchants hold no cryptographic keys, and no balance
          is ever written to the card — so neither impersonation nor tampering
          is open to them.
        </>
      ),
    },
    ledger: {
      t: "An append-only ledger",
      b: "Every movement of money is recorded in a ledger that cannot be edited. Idempotency keys stop double spending, refunds return to the top-up they came from, and any balance can be rebuilt from the ledger at any time.",
    },
    expiry: {
      t: "Six-month expiry, by design",
      b: (
        <>
          Each top-up expires six months later, on the Japanese calendar. The
          exemption for instruments valid for no more than six months from issue
          is implemented strictly, auditably and immutably.
        </>
      ),
    },
    ops: {
      t: "Transactions, refunds and settlement in one place",
      b: "Balances, transactions, refunds, settlement and staff — merchants run the day from one portal, with each role seeing only what it needs.",
    },
  },
  how: {
    eyebrow: "How it works",
    h2: "From a tap to a settled payment.",
    lead: "The terminal only relays frames between the card and the server. The server holds the keys, and the payment is settled in the server's ledger.",
    tap: {
      t: "The card is tapped",
      b: (
        <>
          The merchant terminal polls the FeliCa and reads its IDm / PMm. The
          terminal holds no keys.
        </>
      ),
    },
    auth: {
      t: "The server authenticates",
      b: (
        <>
          The terminal relays the frames; the server drives mutual
          authentication with its keys and ends up with a verified IDi and an
          authenticated session.
        </>
      ),
    },
    settle: {
      t: "The balance settles it",
      b: "That session pays, tops up, reads the balance or refunds — and the ledger records it.",
    },
  },
  terminals: {
    eyebrow: "Terminals",
    h2: "Any terminal, the same ledger.",
    lead: "Authenticated with a merchant API key. Idempotency keys keep a retry from charging twice.",
    download: "Download",
    android: {
      t: "Android app",
      b: "Turns an NFC phone into a tap-and-done POS. A keypad and a confirm button keep a stray tap from becoming a payment.",
      pill: "NFC-F / FeliCa",
    },
    desktop: {
      t: "Desktop terminal",
      b: (
        <>
          For a fixed register with a PaSoRi reader attached. Launches as a
          browser kiosk.
        </>
      ),
      pill: "Win / macOS / Linux",
    },
    api: {
      t: "REST API",
      b: "Mutual authentication, payments, refunds and balance lookups over JSON. Integrate your own terminal or back office directly.",
      pill: "/v1",
    },
  },
  cta: {
    h2: "Thinking about Melon?",
    lead: "Get in touch through our contact page for rollout and pricing. Already a merchant? Sign in from the portal.",
    contact: "Contact us",
    portal: "Merchant portal",
  },
  footer: {
    about: "An online prepaid payment platform built on the FeliCa IDi.",
    product: "Product",
    merchantHead: "Merchants",
    resources: "Resources",
    legal: "Legal",
    portal: "Merchant portal",
    // Japanese only, like the legal texts — say so rather than imply a translation.
    guide: "オペレーションマニュアル — Operations manual (Japanese)",
    contact: "Contact",
    dlDesktop: "Desktop terminal",
    dlAndroid: "Android app",
    // Japanese is the binding text, so say so rather than implying a translation.
    terms: "利用規約 — Terms of Use (Japanese)",
    merchantTerms: "加盟店規約 — Merchant Terms (Japanese)",
    note: "Melon issues and administers a third-party-type prepaid payment instrument under Japanese law.",
  },
};
