"use client";

import { useState } from "react";
import { api } from "@/lib/api";
import type { Store, User } from "@/lib/types";
import { fmtTime } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";
import { useAuth } from "@/components/portal";

export default function MerchantUsersPage() {
  const toast = useToast();
  const { user } = useAuth();
  const state = useAsync<User[]>(() => api.get<User[]>("/v1/users"));
  const stores = useAsync<Store[]>(() => api.get<Store[]>("/v1/stores"));

  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [storeId, setStoreId] = useState("");
  const [curPw, setCurPw] = useState("");
  const [newPw, setNewPw] = useState("");

  const storeName = (id: string | null) =>
    id ? (stores.data?.find((s) => s.id === id)?.name ?? "—") : "全店舗";

  const createStaff = async () => {
    if (!name.trim() || !email.trim() || !password)
      return toast("名前・メール・パスワードを入力してください");
    try {
      await api.post("/v1/users", {
        name: name.trim(),
        email: email.trim(),
        password,
        store_id: storeId || null,
      });
      toast("スタッフを追加しました");
      setName("");
      setEmail("");
      setPassword("");
      setStoreId("");
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const setStaffStatus = async (id: string, status: string) => {
    try {
      await api.post(`/v1/users/${encodeURIComponent(id)}/status`, { status });
      toast(
        status === "active"
          ? "有効化しました"
          : "無効化しました(セッションも失効)",
      );
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const changePassword = async () => {
    if (!curPw || !newPw)
      return toast("現在と新しいパスワードを入力してください");
    try {
      await api.post("/v1/auth/password", {
        current_password: curPw,
        new_password: newPw,
      });
      alert("パスワードを変更しました。再度サインインしてください。");
      window.location.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <>
      <div className="panel">
        <h2>スタッフを追加</h2>
        <p className="muted">
          自店のユーザーのみ作成できます(パスワードは 10 文字以上)。
        </p>
        <div className="row">
          <div className="field">
            <label>名前</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="山田 太郎"
            />
          </div>
          <div className="field">
            <label>メールアドレス</label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="staff@example.com"
            />
          </div>
          <div className="field">
            <label>パスワード</label>
            <input
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </div>
          <div className="field">
            <label>店舗</label>
            <select
              value={storeId}
              onChange={(e) => setStoreId(e.target.value)}
            >
              <option value="">全店舗(加盟店管理者)</option>
              {(stores.data ?? []).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </div>
          <button className="primary" onClick={createStaff}>
            追加
          </button>
        </div>
      </div>

      <div className="panel">
        <h2>自店のユーザー</h2>
        <Async state={state}>
          {(users) => (
            <div className="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>名前</th>
                    <th>メール</th>
                    <th>店舗</th>
                    <th>状態</th>
                    <th>作成日時</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {users.length === 0 ? (
                    <tr>
                      <td colSpan={6} className="empty">
                        ユーザーなし
                      </td>
                    </tr>
                  ) : (
                    users.map((u) => {
                      const self = u.id === user.id;
                      return (
                        <tr key={u.id}>
                          <td>
                            {u.name}
                            {self && <span className="muted"> (あなた)</span>}
                          </td>
                          <td className="mono">{u.email}</td>
                          <td>{storeName(u.store_id)}</td>
                          <td>
                            {u.status === "active" ? (
                              "有効"
                            ) : (
                              <span className="neg">無効</span>
                            )}
                          </td>
                          <td className="muted">{fmtTime(u.created_at)}</td>
                          <td style={{ textAlign: "right" }}>
                            {self ? null : (
                              <button
                                className="sm"
                                onClick={() =>
                                  setStaffStatus(
                                    u.id,
                                    u.status === "active"
                                      ? "disabled"
                                      : "active",
                                  )
                                }
                              >
                                {u.status === "active" ? "無効化" : "有効化"}
                              </button>
                            )}
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
          )}
        </Async>
      </div>

      <div className="panel">
        <h2>パスワード変更</h2>
        <div className="row">
          <div className="field">
            <label>現在のパスワード</label>
            <input
              type="password"
              autoComplete="current-password"
              value={curPw}
              onChange={(e) => setCurPw(e.target.value)}
            />
          </div>
          <div className="field">
            <label>新しいパスワード</label>
            <input
              type="password"
              autoComplete="new-password"
              value={newPw}
              onChange={(e) => setNewPw(e.target.value)}
            />
          </div>
          <button className="primary" onClick={changePassword}>
            変更
          </button>
        </div>
        <p className="muted" style={{ margin: "10px 0 0" }}>
          変更すると全端末のセッションが失効し、再サインインが必要です。
        </p>
      </div>
    </>
  );
}
