"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { api } from "@/lib/api";
import type { LoginResp, Merchant, Role, User } from "@/lib/types";
import { Spinner, errMsg } from "@/components/ui";

export interface Tab {
  href: string;
  label: string;
  /** Hidden for store-scoped merchant users (they manage only their own store). */
  hideForStoreUser?: boolean;
}

interface AuthValue {
  user: User;
  /** The signed-in merchant (merchant portal only); null for admins. */
  merchant: Merchant | null;
  /** Re-fetch the merchant (after a settlement changes, etc.). */
  reloadMerchant: () => Promise<void>;
}

const AuthContext = createContext<AuthValue | null>(null);

export function useAuth(): AuthValue {
  const v = useContext(AuthContext);
  if (!v) throw new Error("useAuth must be used inside a PortalShell");
  return v;
}

type Status = "loading" | "in" | "out";

export function PortalShell({
  role,
  brand,
  tabs,
  children,
}: {
  role: Role;
  brand: string;
  tabs: Tab[];
  children: React.ReactNode;
}) {
  const [status, setStatus] = useState<Status>("loading");
  const [user, setUser] = useState<User | null>(null);
  const [merchant, setMerchant] = useState<Merchant | null>(null);

  const reloadMerchant = useCallback(async () => {
    if (role !== "merchant") return;
    setMerchant(await api.get<Merchant>("/v1/me"));
  }, [role]);

  // Adopt a signed-in user only if their role matches this portal.
  const adopt = useCallback(
    async (u: User) => {
      if (u.role !== role) {
        setStatus("out");
        return role === "admin"
          ? "この画面は発行者(管理者)専用です。"
          : "この画面は加盟店ユーザー専用です。";
      }
      setUser(u);
      if (role === "merchant") setMerchant(await api.get<Merchant>("/v1/me"));
      setStatus("in");
      return null;
    },
    [role],
  );

  // Restore an existing cookie session on first load.
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const u = await api.get<User>("/v1/auth/me");
        if (alive) await adopt(u);
      } catch {
        if (alive) setStatus("out");
      }
    })();
    return () => {
      alive = false;
    };
  }, [adopt]);

  if (status === "loading") return <Spinner />;

  if (status === "out" || !user) {
    return <LoginCard brand={brand} onLoggedIn={adopt} />;
  }

  const who =
    role === "merchant" && merchant
      ? `${user.name} — ${merchant.name}(${merchant.code})`
      : `${user.name}(${user.email})`;

  // Store-scoped merchant users only see their own store — hide admin-only tabs.
  const storeScoped = role === "merchant" && !!user.store_id;
  const visibleTabs = storeScoped
    ? tabs.filter((t) => !t.hideForStoreUser)
    : tabs;

  return (
    <AuthContext.Provider value={{ user, merchant, reloadMerchant }}>
      <Shell brand={brand} tabs={visibleTabs} who={who}>
        {children}
      </Shell>
    </AuthContext.Provider>
  );
}

function Shell({
  brand,
  tabs,
  who,
  children,
}: {
  brand: string;
  tabs: Tab[];
  who: string;
  children: React.ReactNode;
}) {
  const pathname = usePathname();
  const base = tabs[0].href;
  const isActive = (href: string) =>
    href === base
      ? pathname === base
      : pathname === href || pathname.startsWith(href + "/");

  const signOut = async () => {
    try {
      await api.post("/v1/auth/logout");
    } catch {
      /* ignore */
    }
    window.location.reload();
  };

  return (
    <>
      <header className="topbar">
        <div className="brand">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src="/melon-logo.png"
            alt=""
            width={22}
            height={22}
            style={{ borderRadius: 6, verticalAlign: "middle", marginRight: 6 }}
          />
          {brand}
        </div>
        <nav className="tabs">
          {tabs.map((t) => (
            <Link
              key={t.href}
              href={t.href}
              className={isActive(t.href) ? "active" : ""}
            >
              {t.label}
            </Link>
          ))}
        </nav>
        <span className="muted" style={{ fontSize: 12.5 }}>
          {who}
        </span>
        <button onClick={signOut}>サインアウト</button>
      </header>
      <main className="content">{children}</main>
    </>
  );
}

function LoginCard({
  brand,
  onLoggedIn,
}: {
  brand: string;
  onLoggedIn: (u: User) => Promise<string | null>;
}) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!email.trim() || !password) return;
    setBusy(true);
    setError(null);
    try {
      const r = await api.post<LoginResp>("/v1/auth/login", {
        email: email.trim(),
        password,
      });
      const mismatch = await onLoggedIn(r.user);
      if (mismatch) setError(mismatch);
    } catch (e) {
      setError("サインインできません: " + errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-wrap">
      <div className="login-card">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src="/melon-logo.png"
          alt=""
          width={56}
          height={56}
          style={{ display: "block", margin: "0 auto 10px", borderRadius: 14 }}
        />
        <h1>{brand}</h1>
        <p className="muted">アカウントでサインインしてください。</p>
        <input
          type="email"
          placeholder="メールアドレス"
          autoComplete="username"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          onKeyDown={(e) =>
            e.key === "Enter" && document.getElementById("pw")?.focus()
          }
        />
        <input
          id="pw"
          type="password"
          placeholder="パスワード"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
        />
        <button className="primary" onClick={submit} disabled={busy}>
          サインイン
        </button>
        {error && (
          <p
            className="muted"
            style={{ color: "var(--danger)", marginTop: 12 }}
          >
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
