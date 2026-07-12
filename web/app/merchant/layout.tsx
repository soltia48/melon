"use client";

import { PortalShell, type Tab } from "@/components/portal";

const TABS: Tab[] = [
  { href: "/merchant", label: "概要" },
  { href: "/merchant/transactions", label: "取引" },
  { href: "/merchant/users", label: "ユーザー" },
];

export default function MerchantLayout({ children }: { children: React.ReactNode }) {
  return (
    <PortalShell role="merchant" brand="Melon 加盟店ポータル" tabs={TABS}>
      {children}
    </PortalShell>
  );
}
