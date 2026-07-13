"use client";

import { useState } from "react";
import { api } from "@/lib/api";
import type {
  IssuerAdjustResp,
  IssuerAdjustment,
  IssuerBalance,
} from "@/lib/types";
import { fmtTime, yen } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

interface IssuerData {
  balance: IssuerBalance;
  adjustments: IssuerAdjustment[];
}

export default function IssuerPage() {
  const toast = useToast();
  const state = useAsync<IssuerData>(async () => {
    const [balance, adjustments] = await Promise.all([
      api.get<IssuerBalance>("/v1/admin/issuer/balance"),
      api.get<IssuerAdjustment[]>("/v1/admin/issuer/adjustments?limit=50"),
    ]);
    return { balance, adjustments };
  });

  const [sign, setSign] = useState("-1");
  const [amount, setAmount] = useState("");
  const [note, setNote] = useState("");

  const submit = async () => {
    const amt = parseInt(amount, 10);
    if (!amt || amt <= 0) return toast("金額を入力してください");
    try {
      const r = await api.post<IssuerAdjustResp>("/v1/admin/issuer/adjust", {
        delta: parseInt(sign, 10) * amt,
        reason: note.trim() || null,
      });
      toast(`記帳しました: 発行者残高 ${yen(r.balance)}`);
      setAmount("");
      setNote("");
      state.reload();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <Async state={state}>
      {({ balance: b, adjustments }) => (
        <>
          <div className="cards">
            <div className="stat">
              <div className="label">発行者残高(収益)</div>
              <div className={"value" + (b.balance < 0 ? " neg" : "")}>
                {yen(b.balance)}
              </div>
            </div>
            <div className="stat">
              <div className="label">決済手数料収入</div>
              <div className="value">{yen(b.fee_income)}</div>
            </div>
            <div className="stat">
              <div className="label">消滅済み残高(失効益)</div>
              <div className="value">{yen(b.expiry_income)}</div>
            </div>
            <div className="stat">
              <div className="label">引き出し・補正合計</div>
              <div className={"value" + (b.adjustments < 0 ? " neg" : "")}>
                {yen(b.adjustments)}
              </div>
            </div>
          </div>

          <div className="panel">
            <h2>発行者残高</h2>
            <p className="muted">
              発行者残高 = 決済手数料収入 + 消滅済み残高(失効益) +
              引き出し・補正。会計上の収益残高で、現金はチャージを取り扱った加盟店が保持します。
            </p>
            <h3 style={{ margin: "14px 0 8px" }}>
              引き出し・補正(監査記録に残ります)
            </h3>
            <div className="row" style={{ marginBottom: 6 }}>
              <div className="field">
                <label>種別</label>
                <select value={sign} onChange={(e) => setSign(e.target.value)}>
                  <option value="-1">引き出し(withdrawal)</option>
                  <option value="1">補正・注入(credit)</option>
                </select>
              </div>
              <div className="field">
                <label>金額(円)</label>
                <input
                  type="number"
                  min={1}
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  placeholder="10000"
                />
              </div>
              <div className="field" style={{ flex: 1, minWidth: 160 }}>
                <label>備考</label>
                <input
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  placeholder="理由・振込先など(任意)"
                />
              </div>
              <button className="primary" onClick={submit}>
                適用
              </button>
            </div>
          </div>

          <div className="panel">
            <h2>引き出し・補正の履歴</h2>
            <div className="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>日時</th>
                    <th>種別</th>
                    <th className="num">金額</th>
                    <th>備考</th>
                  </tr>
                </thead>
                <tbody>
                  {adjustments.length === 0 ? (
                    <tr>
                      <td colSpan={4} className="empty">
                        記録なし
                      </td>
                    </tr>
                  ) : (
                    adjustments.map((a) => (
                      <tr key={a.id}>
                        <td className="muted">{fmtTime(a.created_at)}</td>
                        <td className="kind">
                          {a.amount < 0 ? "引き出し" : "補正・注入"}
                        </td>
                        <td className={"num " + (a.amount < 0 ? "neg" : "pos")}>
                          {yen(a.amount)}
                        </td>
                        <td>{a.note || ""}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </>
      )}
    </Async>
  );
}
