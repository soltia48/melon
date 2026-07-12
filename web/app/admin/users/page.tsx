"use client";

import { useState } from "react";
import { api } from "@/lib/api";
import type { Merchant, User } from "@/lib/types";
import { fmtTime } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

interface UsersData {
  users: User[];
  merchants: Merchant[];
}

export default function AdminUsersPage() {
  const toast = useToast();
  const state = useAsync<UsersData>(async () => {
    const [users, merchants] = await Promise.all([
      api.get<User[]>("/v1/admin/users"),
      api.get<Merchant[]>("/v1/merchants"),
    ]);
    return { users, merchants };
  });

  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<"merchant" | "admin">("merchant");
  const [merchantId, setMerchantId] = useState("");

  const create = async (merchants: Merchant[]) => {
    if (!name.trim() || !email.trim() || !password) return toast("名前・メール・パスワードを入力してください");
    const mid = role === "merchant" ? merchantId || merchants[0]?.id : null;
    if (role === "merchant" && !mid) return toast("加盟店を選択してください");
    try {
      await api.post("/v1/admin/users", {
        name: name.trim(),
        email: email.trim(),
        password,
        role,
        merchant_id: mid,
      });
      toast("ユーザーを作成しました");
      setName("");
      setEmail("");
      setPassword("");
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const setStatus = async (id: string, status: string) => {
    try {
      await api.post(`/v1/admin/users/${encodeURIComponent(id)}/status`, { status });
      toast(status === "active" ? "有効化しました" : "無効化しました(セッションも失効)");
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const resetPassword = async (id: string, mail: string) => {
    const pw = prompt(`${mail} の新しいパスワード(10 文字以上)`);
    if (pw === null) return;
    try {
      await api.post(`/v1/admin/users/${encodeURIComponent(id)}/password`, { new_password: pw });
      toast("パスワードを再設定しました(既存セッションは失効)");
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <Async state={state}>
      {({ users, merchants }) => {
        const codeOf = (id: string | null) => merchants.find((m) => m.id === id)?.code || "—";
        return (
          <>
            <div className="panel">
              <h2>ユーザーを追加</h2>
              <p className="muted">
                発行者(管理画面)ユーザー、または任意の加盟店のユーザーを作成します。パスワードは 10 文字以上。
              </p>
              <div className="row">
                <div className="field">
                  <label>名前</label>
                  <input value={name} onChange={(e) => setName(e.target.value)} placeholder="山田 太郎" />
                </div>
                <div className="field">
                  <label>メールアドレス</label>
                  <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="user@example.com" />
                </div>
                <div className="field">
                  <label>パスワード</label>
                  <input type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
                </div>
                <div className="field">
                  <label>種別</label>
                  <select value={role} onChange={(e) => setRole(e.target.value as "merchant" | "admin")}>
                    <option value="merchant">加盟店ユーザー</option>
                    <option value="admin">発行者(管理者)</option>
                  </select>
                </div>
                {role === "merchant" && (
                  <div className="field">
                    <label>加盟店</label>
                    <select value={merchantId} onChange={(e) => setMerchantId(e.target.value)}>
                      {merchants.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.code} — {m.name}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
                <button className="primary" onClick={() => create(merchants)}>
                  作成
                </button>
              </div>
            </div>

            <div className="panel">
              <h2>ユーザー一覧</h2>
              <div className="table-wrap">
                <table>
                  <thead>
                    <tr>
                      <th>名前</th>
                      <th>メール</th>
                      <th>種別</th>
                      <th>加盟店</th>
                      <th>状態</th>
                      <th>作成日時</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {users.length === 0 ? (
                      <tr>
                        <td colSpan={7} className="empty">
                          ユーザーなし
                        </td>
                      </tr>
                    ) : (
                      users.map((u) => (
                        <tr key={u.id}>
                          <td>{u.name}</td>
                          <td className="mono">{u.email}</td>
                          <td>
                            <span className="kind">{u.role === "admin" ? "発行者" : "加盟店"}</span>
                          </td>
                          <td className="mono muted">{u.role === "merchant" ? codeOf(u.merchant_id) : "—"}</td>
                          <td>{u.status === "active" ? "有効" : <span className="neg">無効</span>}</td>
                          <td className="muted">{fmtTime(u.created_at)}</td>
                          <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                            <button
                              className="sm"
                              onClick={() => setStatus(u.id, u.status === "active" ? "disabled" : "active")}
                            >
                              {u.status === "active" ? "無効化" : "有効化"}
                            </button>
                            <button className="sm" onClick={() => resetPassword(u.id, u.email)}>
                              パスワード再設定
                            </button>
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </>
        );
      }}
    </Async>
  );
}
