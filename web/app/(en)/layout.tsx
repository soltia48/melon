import type { Metadata } from "next";
import "../globals.css";
import { ToastProvider } from "@/components/toast";
import { SITE_NAME, SITE_URL } from "@/lib/site";

/**
 * Root layout for the English landing page. It exists separately from `(ja)` for
 * one reason: `<html lang>`. Only a root layout renders `<html>`, so a page can
 * only be served as English if it has a root layout of its own.
 */
const TITLE = "Melon — online prepaid payments on FeliCa";

const DESCRIPTION =
  "No app to install, no card to issue. The FeliCa your customers already carry becomes the payment method — tap to pay, top up or check a balance.";

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: { default: TITLE, template: "%s | Melon" },
  description: DESCRIPTION,
  openGraph: {
    type: "website",
    locale: "en_US",
    alternateLocale: "ja_JP",
    siteName: SITE_NAME,
    url: `${SITE_URL}/en`,
    title: TITLE,
    description: DESCRIPTION,
  },
  twitter: { card: "summary_large_image" },
};

export default function EnRootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>
        <ToastProvider>{children}</ToastProvider>
      </body>
    </html>
  );
}
