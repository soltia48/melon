/**
 * Canonical public identity of the site. `metadataBase` needs an absolute origin
 * to turn the Open Graph image paths into absolute URLs (crawlers reject relative
 * ones), and metadata is baked at build time — so this is the production origin,
 * overridable at build with NEXT_PUBLIC_SITE_URL for a staging deploy.
 */
export const SITE_URL =
  process.env.NEXT_PUBLIC_SITE_URL ?? "https://melon.unknowntech.jp";

export const SITE_HOST = new URL(SITE_URL).host;

export const SITE_NAME = "Melon";

/**
 * hreflang map for the landing page, which is published in both languages.
 * Japanese is x-default: this is a Japanese payment instrument, so a visitor
 * whose language we cannot place belongs on the Japanese page.
 */
export const LANGUAGE_ALTERNATES = {
  ja: "/",
  en: "/en",
  "x-default": "/",
};

export const SITE_DESCRIPTION =
  "専用アプリのインストールも、専用カードの発行も不要。お手持ちの FeliCa をかざすだけで、支払い・チャージ・残高照会。オンライン前払式支払手段プラットフォームです。";
