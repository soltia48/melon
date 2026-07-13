"use client";

import { useState } from "react";
import { api } from "@/lib/api";
import type { ApiKey, CreateApiKeyResp, Store } from "@/lib/types";
import { fmtTime } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

export default function MerchantStoresPage() {
  const state = useAsync<Store[]>(() => api.get<Store[]>("/v1/stores"));

  return (
    <>
      <div className="panel">
        <h2>店舗</h2>
        <p className="muted">
          店舗の追加・名称変更は発行者(管理者)が行います。ここでは各店舗の端末用
          API キーを発行・失効できます。
        </p>
      </div>
      <Async state={state}>
        {(stores) =>
          stores.length === 0 ? (
            <div className="panel">
              <div className="empty">店舗がありません</div>
            </div>
          ) : (
            <>
              {stores.map((s) => (
                <StorePanel key={s.id} store={s} />
              ))}
            </>
          )
        }
      </Async>
    </>
  );
}

function StorePanel({ store }: { store: Store }) {
  const toast = useToast();
  const keys = useAsync<ApiKey[]>(() =>
    api.get<ApiKey[]>(`/v1/stores/${store.id}/api-keys`),
  );
  const [label, setLabel] = useState("");
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const issue = async () => {
    setBusy(true);
    try {
      const r = await api.post<CreateApiKeyResp>(
        `/v1/stores/${store.id}/api-keys`,
        { label: label.trim() || null },
      );
      setRevealed(r.api_key);
      setLabel("");
      keys.reload();
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: string) => {
    if (
      !confirm(
        "この API キーを失効します。使用中の端末は認証できなくなります。",
      )
    )
      return;
    try {
      await api.del(
        `/v1/stores/${store.id}/api-keys/${encodeURIComponent(id)}`,
      );
      toast("失効しました");
      keys.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <div className="panel">
      <h2 style={{ display: "flex", alignItems: "center", gap: 10 }}>
        {store.name}
        <span className={"pill " + store.status}>{store.status}</span>
        {store.is_default && (
          <span className="muted" style={{ fontSize: 13 }}>
            既定店舗
          </span>
        )}
        <span
          className="mono muted"
          style={{ fontSize: 13, marginLeft: "auto" }}
        >
          {store.code}
        </span>
      </h2>

      {revealed && (
        <div
          className="panel"
          style={{ background: "var(--accent-weak)", margin: "0 0 14px" }}
        >
          <p style={{ margin: "0 0 6px", fontWeight: 700 }}>
            新しい API キー(この画面を離れると再表示できません)
          </p>
          <code className="mono" style={{ wordBreak: "break-all" }}>
            {revealed}
          </code>
          <div style={{ marginTop: 10 }}>
            <button
              className="sm"
              onClick={() => {
                navigator.clipboard?.writeText(revealed);
                toast("コピーしました");
              }}
            >
              コピー
            </button>{" "}
            <button className="sm" onClick={() => setRevealed(null)}>
              閉じる
            </button>
          </div>
        </div>
      )}

      <div className="row" style={{ marginBottom: 12 }}>
        <div className="field">
          <label>ラベル(任意)</label>
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="レジ1 など"
          />
        </div>
        <button className="primary" onClick={issue} disabled={busy}>
          API キーを発行
        </button>
      </div>

      <Async state={keys}>
        {(list) => (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>ラベル</th>
                  <th>状態</th>
                  <th>発行日時</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {list.length === 0 ? (
                  <tr>
                    <td colSpan={4} className="empty">
                      API キーがありません
                    </td>
                  </tr>
                ) : (
                  list.map((k) => (
                    <tr key={k.id}>
                      <td>
                        {k.label || <span className="muted">(なし)</span>}
                      </td>
                      <td>
                        {k.active ? "有効" : <span className="neg">失効</span>}
                      </td>
                      <td className="muted">{fmtTime(k.created_at)}</td>
                      <td style={{ textAlign: "right" }}>
                        {k.active && (
                          <button className="sm" onClick={() => revoke(k.id)}>
                            失効
                          </button>
                        )}
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>
        )}
      </Async>
    </div>
  );
}
