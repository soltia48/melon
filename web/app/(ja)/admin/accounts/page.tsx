"use client";

import { useState } from "react";
import { api, qs } from "@/lib/api";
import type {
  AccountRow,
  AdminBalance,
  AdminRefundable,
  AdminTxn,
} from "@/lib/types";
import { fmtTime, scHex, yen } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

interface AcctKey {
  sc: string;
  idm: string;
  idi: string;
}

export default function AccountsPage() {
  const state = useAsync<AccountRow[]>(() =>
    api.get<AccountRow[]>("/v1/admin/accounts"),
  );
  const [selected, setSelected] = useState<AcctKey | null>(null);

  const [sc, setSc] = useState("");
  const [idm, setIdm] = useState("");
  const [idi, setIdi] = useState("");
  const lookup = () => {
    if (!sc.trim() || !idm.trim() || !idi.trim()) return;
    setSelected({ sc: sc.trim(), idm: idm.trim(), idi: idi.trim() });
  };

  return (
    <>
      <Async state={state}>
        {(accounts) => (
          <div className="panel">
            <h2>
              利用者一覧({accounts.length}){" "}
              <span className="muted" style={{ fontWeight: 400 }}>
                行を選択して残高を確認・編集(System Code・IDm・IDi
                の三つ組で識別)
              </span>
            </h2>
            <div className="row" style={{ marginBottom: 12 }}>
              <div className="field">
                <label>System Code</label>
                <input
                  className="mono"
                  placeholder="0x0003"
                  style={{ width: 110 }}
                  value={sc}
                  onChange={(e) => setSc(e.target.value)}
                />
              </div>
              <div className="field">
                <label>IDm(16桁hex)</label>
                <input
                  className="mono"
                  placeholder="0102030405060708"
                  value={idm}
                  onChange={(e) => setIdm(e.target.value)}
                />
              </div>
              <div className="field">
                <label>IDi(16桁hex)</label>
                <input
                  className="mono"
                  placeholder="1122334455667788"
                  value={idi}
                  onChange={(e) => setIdi(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && lookup()}
                />
              </div>
              <button onClick={lookup}>照会</button>
            </div>
            <div className="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>System</th>
                    <th>IDm</th>
                    <th>IDi</th>
                    <th>状態</th>
                    <th className="num">残高</th>
                    <th>作成日時</th>
                  </tr>
                </thead>
                <tbody>
                  {accounts.length === 0 ? (
                    <tr>
                      <td colSpan={6} className="empty">
                        利用者がいません
                      </td>
                    </tr>
                  ) : (
                    accounts.map((a) => (
                      <tr
                        key={`${a.system_code}/${a.idm}/${a.idi}`}
                        className="clickable"
                        onClick={() =>
                          setSelected({
                            sc: String(a.system_code),
                            idm: a.idm,
                            idi: a.idi,
                          })
                        }
                      >
                        <td className="mono">{scHex(a.system_code)}</td>
                        <td className="mono">{a.idm}</td>
                        <td className="mono">{a.idi}</td>
                        <td>
                          <span className={"pill " + a.status}>{a.status}</span>
                        </td>
                        <td className="num">{yen(a.balance)}</td>
                        <td className="muted">{fmtTime(a.created_at)}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </Async>

      {selected && (
        <AccountDetail
          key={`${selected.sc}/${selected.idm}/${selected.idi}`}
          acct={selected}
          onAccountsChanged={state.reload}
        />
      )}
    </>
  );
}

interface Detail {
  bal: AdminBalance;
  txns: AdminTxn[];
  refundable: AdminRefundable[];
}

function AccountDetail({
  acct,
  onAccountsChanged,
}: {
  acct: AcctKey;
  onAccountsChanged: () => Promise<void>;
}) {
  const toast = useToast();
  const path = `/v1/admin/accounts/${encodeURIComponent(acct.sc)}/${encodeURIComponent(acct.idm)}/${encodeURIComponent(acct.idi)}`;
  const filter = qs({
    limit: 20,
    system_code: acct.sc,
    idm: acct.idm,
    idi: acct.idi,
  });

  const state = useAsync<Detail>(async () => {
    const [bal, txns, refundable] = await Promise.all([
      api.get<AdminBalance>(`${path}/balance`),
      api.get<AdminTxn[]>("/v1/admin/transactions" + filter),
      api.get<AdminRefundable[]>("/v1/admin/refundable" + filter),
    ]);
    return { bal, txns, refundable };
  }, [acct.sc, acct.idm, acct.idi]);

  const [adjSign, setAdjSign] = useState("1");
  const [adjAmount, setAdjAmount] = useState("");
  const [adjReason, setAdjReason] = useState("");

  const refresh = async () => {
    await state.reload();
    await onAccountsChanged();
  };

  const submitAdjust = async () => {
    const amount = parseInt(adjAmount, 10);
    if (!amount || amount <= 0) return toast("金額を入力してください");
    try {
      const r = await api.post<{ balance: number }>(`${path}/adjust`, {
        delta: parseInt(adjSign, 10) * amount,
        reason: adjReason.trim() || null,
      });
      toast(`調整完了: 残高 ${yen(r.balance)}`);
      setAdjAmount("");
      setAdjReason("");
      refresh();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const refund = async (paymentId: string, refundable: number) => {
    const input = prompt(`返金額(円)。空欄で全額(${refundable})を返金します。`);
    if (input === null) return;
    const amount = input.trim() === "" ? null : parseInt(input, 10);
    if (amount !== null && !(amount > 0)) return toast("金額が不正です");
    try {
      const r = await api.post<{ amount: number }>("/v1/admin/refunds", {
        payment_id: paymentId,
        amount,
      });
      toast(`返金しました: ${yen(r.amount)}`);
      refresh();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const voidPayment = async (paymentId: string) => {
    if (!confirm("この支払いを全額取消しますか?")) return;
    try {
      const r = await api.post<{ amount: number }>(
        `/v1/admin/payments/${encodeURIComponent(paymentId)}/void`,
      );
      toast(`取消しました: ${yen(r.amount)}`);
      refresh();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  return (
    <Async state={state}>
      {({ bal, txns, refundable }) => (
        <div className="panel">
          <h2>
            利用者詳細{" "}
            <span className="mono" style={{ fontWeight: 400 }}>
              {scHex(bal.system_code)} / {bal.idm} : {bal.idi}
            </span>
          </h2>
          <div className="cards" style={{ marginBottom: 16 }}>
            <div className="stat">
              <div className="label">利用可能残高</div>
              <div className="value">{yen(bal.total)}</div>
            </div>
          </div>

          <h3 style={{ marginBottom: 8 }}>
            残高の調整(台帳に adjustment として記帳)
          </h3>
          <div className="row" style={{ marginBottom: 6 }}>
            <div className="field">
              <label>種別</label>
              <select
                value={adjSign}
                onChange={(e) => setAdjSign(e.target.value)}
              >
                <option value="1">入金(credit)</option>
                <option value="-1">引落(debit)</option>
              </select>
            </div>
            <div className="field">
              <label>金額(円)</label>
              <input
                type="number"
                min={1}
                value={adjAmount}
                onChange={(e) => setAdjAmount(e.target.value)}
                placeholder="1000"
              />
            </div>
            <div className="field" style={{ flex: 1, minWidth: 160 }}>
              <label>理由</label>
              <input
                value={adjReason}
                onChange={(e) => setAdjReason(e.target.value)}
                placeholder="調整理由(任意)"
              />
            </div>
            <button className="primary" onClick={submitAdjust}>
              適用
            </button>
          </div>
          <p className="muted" style={{ margin: "0 0 18px" }}>
            入金は新しい
            6ヶ月バケットを作成、引落は期限が近い順に消費(残高不足は不可)。
          </p>

          <h3 style={{ marginBottom: 8 }}>返金可能な支払い</h3>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>日時</th>
                  <th className="num">支払額</th>
                  <th className="num">返金可能額</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {refundable.length === 0 ? (
                  <tr>
                    <td colSpan={4} className="empty">
                      返金可能な支払いなし
                    </td>
                  </tr>
                ) : (
                  refundable.map((p) => (
                    <tr key={p.id}>
                      <td className="muted">{fmtTime(p.occurred_at)}</td>
                      <td className="num">{yen(p.amount)}</td>
                      <td className="num">{yen(p.refundable)}</td>
                      <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                        <button
                          className="sm"
                          onClick={() => refund(p.id, p.refundable)}
                        >
                          返金
                        </button>
                        <button
                          className="sm"
                          onClick={() => voidPayment(p.id)}
                        >
                          取消
                        </button>
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>

          <h3 style={{ margin: "18px 0 8px" }}>有効なバケット</h3>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>バケット</th>
                  <th className="num">残高</th>
                  <th>失効日時</th>
                </tr>
              </thead>
              <tbody>
                {bal.buckets.length === 0 ? (
                  <tr>
                    <td colSpan={3} className="empty">
                      有効なバケットなし
                    </td>
                  </tr>
                ) : (
                  bal.buckets.map((b) => (
                    <tr key={b.bucket_id}>
                      <td className="mono muted">{b.bucket_id.slice(0, 8)}…</td>
                      <td className="num">{yen(b.remaining)}</td>
                      <td className="muted">{fmtTime(b.expires_at)}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>

          <h3 style={{ margin: "18px 0 8px" }}>最近の取引</h3>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>日時</th>
                  <th>種別</th>
                  <th className="num">金額</th>
                </tr>
              </thead>
              <tbody>
                {txns.length === 0 ? (
                  <tr>
                    <td colSpan={3} className="empty">
                      履歴なし
                    </td>
                  </tr>
                ) : (
                  txns.map((t) => {
                    const sign =
                      t.kind === "payment" || t.kind === "expiry"
                        ? "neg"
                        : "pos";
                    return (
                      <tr key={t.id}>
                        <td className="muted">{fmtTime(t.occurred_at)}</td>
                        <td className="kind">{t.kind}</td>
                        <td className={"num " + sign}>{yen(t.amount)}</td>
                      </tr>
                    );
                  })
                )}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </Async>
  );
}
