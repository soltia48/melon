"use client";

import { useState } from "react";
import { api, qs } from "@/lib/api";
import type { MerchantTxn } from "@/lib/types";
import { fmtTime, shortId, yen } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

const KINDS = ["", "payment", "top_up", "refund", "reversal"];

export default function MerchantTransactionsPage() {
  const toast = useToast();
  const [kind, setKind] = useState("");
  const [limit, setLimit] = useState("100");
  const [query, setQuery] = useState({ kind: "", limit: "100" });

  const state = useAsync<MerchantTxn[]>(
    () => api.get<MerchantTxn[]>("/v1/transactions" + qs({ limit: query.limit || 100, kind: query.kind })),
    [query],
  );

  const refund = async (paymentId: string) => {
    const input = prompt("返金額(円)。空欄で全額返金します。");
    if (input === null) return;
    const trimmed = input.trim();
    let amount: number | null = null;
    if (trimmed !== "") {
      amount = parseInt(trimmed, 10);
      if (!(amount > 0)) return toast("正の金額を入力してください");
    }
    try {
      const r = await api.post<{ amount: number }>("/v1/refunds", { payment_id: paymentId, amount });
      toast(`返金しました: ${yen(r.amount)}`);
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const voidPayment = async (paymentId: string) => {
    if (!confirm("この支払いを取り消します(全額)。よろしいですか?")) return;
    try {
      const r = await api.post<{ amount: number }>(`/v1/payments/${encodeURIComponent(paymentId)}/void`);
      toast(`取り消しました: ${yen(r.amount)}`);
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <>
      <div className="panel">
        <h2>取引</h2>
        <div className="row">
          <div className="field">
            <label>種別</label>
            <select value={kind} onChange={(e) => setKind(e.target.value)}>
              {KINDS.map((k) => (
                <option key={k} value={k}>
                  {k || "すべて"}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>件数</label>
            <input type="number" min={1} max={500} value={limit} onChange={(e) => setLimit(e.target.value)} style={{ width: 90 }} />
          </div>
          <button className="primary" onClick={() => setQuery({ kind, limit })}>
            更新
          </button>
        </div>
      </div>

      <div className="panel">
        <div className="table-wrap">
          <Async state={state}>
            {(list) =>
              list.length === 0 ? (
                <div className="empty">取引がありません</div>
              ) : (
                <table>
                  <thead>
                    <tr>
                      <th>日時</th>
                      <th>種別</th>
                      <th>利用者 ID(仮名)</th>
                      <th className="num">金額</th>
                      <th className="num">手数料</th>
                      <th>メモ</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {list.map((t) => {
                      const sign =
                        t.kind === "payment"
                          ? "pos"
                          : t.kind === "top_up" || t.kind === "refund" || t.kind === "reversal"
                            ? "neg"
                            : "";
                      const disp = t.kind === "payment" ? yen(t.amount) : "−" + yen(t.amount).slice(1);
                      return (
                        <tr key={t.id}>
                          <td className="muted">{fmtTime(t.occurred_at)}</td>
                          <td>{t.kind}</td>
                          <td className="mono" title={t.account_id}>
                            {shortId(t.account_id)}
                          </td>
                          <td className={"num " + sign}>{disp}</td>
                          <td className="num muted">{t.fee ? yen(t.fee) : "—"}</td>
                          <td className="muted">
                            <span
                              title={t.note ?? undefined}
                              style={{
                                display: "inline-block",
                                maxWidth: 220,
                                overflow: "hidden",
                                textOverflow: "ellipsis",
                                whiteSpace: "nowrap",
                                verticalAlign: "bottom",
                              }}
                            >
                              {t.note || "—"}
                            </span>
                          </td>
                          <td>
                            {t.kind === "payment" ? (
                              <>
                                <button className="sm" onClick={() => refund(t.id)}>
                                  返金
                                </button>
                                <button className="sm" onClick={() => voidPayment(t.id)}>
                                  取消
                                </button>
                              </>
                            ) : (
                              "—"
                            )}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )
            }
          </Async>
        </div>
      </div>
    </>
  );
}
