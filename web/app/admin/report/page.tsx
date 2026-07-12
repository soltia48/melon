"use client";

import { useState } from "react";
import { api, qs } from "@/lib/api";
import type { OutstandingReport } from "@/lib/types";
import { fmtTime, yen } from "@/lib/format";
import { Async, useAsync } from "@/components/ui";

export default function ReportPage() {
  const [asOfInput, setAsOfInput] = useState("");
  const [asOf, setAsOf] = useState<string>(""); // committed ISO string

  const state = useAsync<OutstandingReport>(
    () => api.get<OutstandingReport>("/v1/admin/reports/outstanding-balance" + qs({ as_of: asOf })),
    [asOf],
  );

  const run = () => {
    setAsOf(asOfInput ? new Date(asOfInput).toISOString() : "");
  };

  return (
    <>
      <div className="panel">
        <h2>未使用残高レポート</h2>
        <p className="muted">資金決済法の基準日(3/31・9/30)時点の未使用残高集計に利用できます。</p>
        <div className="row">
          <div className="field">
            <label>基準日時(空欄=現在)</label>
            <input type="datetime-local" value={asOfInput} onChange={(e) => setAsOfInput(e.target.value)} />
          </div>
          <button className="primary" onClick={run}>
            集計
          </button>
        </div>
      </div>

      <Async state={state}>
        {(r) => {
          const max = Math.max(1, ...r.by_expiry_month.map((m) => m.amount));
          return (
            <>
              <div className="cards">
                <div className="stat">
                  <div className="label">未使用残高(合計)</div>
                  <div className="value">{yen(r.total)}</div>
                </div>
                <div className="stat">
                  <div className="label">口座数</div>
                  <div className="value">{r.account_count.toLocaleString()}</div>
                </div>
                <div className="stat">
                  <div className="label">基準日時</div>
                  <div className="value" style={{ fontSize: 16 }}>
                    {fmtTime(r.as_of)}
                  </div>
                </div>
              </div>
              <div className="panel">
                <h2>失効月別内訳</h2>
                <div className="table-wrap">
                  <table>
                    <thead>
                      <tr>
                        <th>失効月(JST)</th>
                        <th>構成</th>
                        <th className="num">金額</th>
                      </tr>
                    </thead>
                    <tbody>
                      {r.by_expiry_month.length === 0 ? (
                        <tr>
                          <td colSpan={3} className="empty">
                            該当なし
                          </td>
                        </tr>
                      ) : (
                        r.by_expiry_month.map((m) => (
                          <tr key={m.month}>
                            <td className="mono">{m.month}</td>
                            <td style={{ width: "55%" }}>
                              <div className="bar" style={{ width: `${((m.amount / max) * 100).toFixed(1)}%` }} />
                            </td>
                            <td className="num">{yen(m.amount)}</td>
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
    </>
  );
}
