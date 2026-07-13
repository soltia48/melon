"use client";

import { PortalShell, type Tab } from "@/components/portal";

const TABS: Tab[] = [
  { href: "/admin", label: "概要" },
  { href: "/admin/merchants", label: "加盟店" },
  { href: "/admin/accounts", label: "利用者" },
  { href: "/admin/transactions", label: "取引" },
  { href: "/admin/report", label: "未使用残高" },
  { href: "/admin/issuer", label: "発行者" },
  { href: "/admin/users", label: "ユーザー" },
];

export default function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <PortalShell role="admin" brand="Melon 管理画面" tabs={TABS}>
      {children}
    </PortalShell>
  );
}
