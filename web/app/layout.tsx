import type { Metadata } from "next";
import "./globals.css";
import { ToastProvider } from "@/components/toast";

export const metadata: Metadata = {
  title: "Melon",
  description: "Melon — オンライン前払式支払手段プラットフォーム",
};

export default function RootLayout({
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
