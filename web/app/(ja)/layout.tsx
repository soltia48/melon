import type { Metadata } from "next";
import "../globals.css";
import { ToastProvider } from "@/components/toast";
import { SITE_DESCRIPTION, SITE_NAME, SITE_URL } from "@/lib/site";

/**
 * Root layout for everything Japanese: the landing page, the legal documents and
 * both consoles. The English landing page has its own root layout under
 * `app/(en)` — a nested layout cannot change `<html lang>`, and serving English
 * under `lang="ja"` misleads both crawlers and screen readers.
 */
const TITLE = "Melon — オンライン前払式支払手段プラットフォーム";

export const metadata: Metadata = {
  // Crawlers reject relative image URLs, so Open Graph needs an absolute origin
  // to resolve them against. The images come from the `opengraph-image.tsx`
  // files, which Next wires into og:image / twitter:image automatically.
  metadataBase: new URL(SITE_URL),
  title: { default: TITLE, template: "%s | Melon" },
  description: SITE_DESCRIPTION,
  openGraph: {
    type: "website",
    locale: "ja_JP",
    alternateLocale: "en_US",
    siteName: SITE_NAME,
    url: SITE_URL,
    title: TITLE,
    description: SITE_DESCRIPTION,
  },
  twitter: { card: "summary_large_image" },
};

export default function JaRootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="ja">
      <body>
        <ToastProvider>{children}</ToastProvider>
      </body>
    </html>
  );
}
