"use client";

import { useState } from "react";
import { api, qs } from "@/lib/api";
import type { AdminTxn } from "@/lib/types";
import { fmtTime, scHex, yen } from "@/lib/format";
import { Async, useAsync } from "@/components/ui";

const KINDS = ["", "top_up", "payment", "refund", "reversal", "adjustment"];

interface Query {
  kind: string;
  sc: string;
  idm: string;
  idi: string;
  limit: string;
}

export default function AdminTransactionsPage() {
  const [kind, setKind] = useState("");
  const [sc, setSc] = useState("");
  const [idm, setIdm] = useState("");
  const [idi, setIdi] = useState("");
  const [limit, setLimit] = useState("50");
  const [query, setQuery] = useState<Query>({ kind: "", sc: "", idm: "", idi: "", limit: "50" });

  const state = useAsync<AdminTxn[]>(
    () =>
      api.get<AdminTxn[]>(
        "/v1/admin/transactions" +
          qs({
            limit: query.limit || 50,
            kind: query.kind,
            system_code: query.sc,
            idm: query.idm,
            idi: query.idi,
          }),
      ),
    [query],
  );

  const search = () => setQuery({ kind, sc: sc.trim(), idm: idm.trim(), idi: idi.trim(), limit });

  return (
    <>
      <div className="panel">
        <h2>取引フィルタ</h2>
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
            <label>System Code</label>
            <input className="mono" placeholder="任意 (例 0x0003)" style={{ width: 130 }} value={sc} onChange={(e) => setSc(e.target.value)} />
          </div>
          <div className="field">
            <label>IDm(16桁hex)</label>
            <input className="mono" placeholder="任意" value={idm} onChange={(e) => setIdm(e.target.value)} />
          </div>
          <div className="field">
            <label>IDi(16桁hex)</label>
            <input className="mono" placeholder="任意" value={idi} onChange={(e) => setIdi(e.target.value)} />
          </div>
          <div className="field">
            <label>件数</label>
            <input type="number" min={1} max={500} value={limit} onChange={(e) => setLimit(e.target.value)} style={{ width: 90 }} />
          </div>
          <button className="primary" onClick={search}>
            検索
          </button>
        </div>
        <p className="muted" style={{ margin: "8px 0 0" }}>
          口座で絞り込む場合は System Code・IDm・IDi をすべて指定してください(三つ組で一意)。
        </p>
      </div>

      <div className="panel">
        <div className="table-wrap">
          <Async state={state}>
            {(list) =>
              list.length === 0 ? (
                <div className="empty">該当する取引がありません</div>
              ) : (
                <table>
                  <thead>
                    <tr>
                      <th>日時</th>
                      <th>種別</th>
                      <th>System</th>
                      <th>IDm</th>
                      <th>IDi</th>
                      <th className="num">金額</th>
                      <th>加盟店</th>
                      <th>メモ</th>
                    </tr>
                  </thead>
                  <tbody>
                    {list.map((t) => {
                      const sign = t.kind === "payment" || t.kind === "expiry" ? "neg" : "pos";
                      return (
                        <tr key={t.id}>
                          <td className="muted">{fmtTime(t.occurred_at)}</td>
                          <td className="kind">{t.kind}</td>
                          <td className="mono">{scHex(t.system_code)}</td>
                          <td className="mono">{t.idm}</td>
                          <td className="mono">{t.idi}</td>
                          <td className={"num " + sign}>{yen(t.amount)}</td>
                          <td className="mono muted">{t.merchant_id ? t.merchant_id.slice(0, 8) + "…" : "—"}</td>
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
