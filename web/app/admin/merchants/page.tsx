"use client";

import { useState } from "react";
import { api, qs } from "@/lib/api";
import type {
  AdminTxn,
  CreateMerchantResp,
  Merchant,
  MerchantAdjustResp,
  RotateKeyResp,
} from "@/lib/types";
import { fmtTime, pct, yen } from "@/lib/format";
import { Async, Modal, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

const STATUSES = ["active", "suspended", "closed"];

interface KeyReveal {
  title: string;
  secret: string;
  merchantId: string;
}

export default function MerchantsPage() {
  const toast = useToast();
  const state = useAsync<Merchant[]>(() => api.get<Merchant[]>("/v1/merchants"));
  const [selected, setSelected] = useState<string | null>(null);
  const [keyReveal, setKeyReveal] = useState<KeyReveal | null>(null);

  return (
    <>
      <CreateMerchant
        onCreated={async (r, code) => {
          await state.reload();
          setKeyReveal({ title: "API キー(新規加盟店)", secret: r.api_key, merchantId: r.merchant_id });
          toast(`加盟店 ${code} を作成しました`);
        }}
      />

      <Async state={state}>
        {(merchants) => (
          <>
            <div className="panel">
              <h2>
                加盟店一覧({merchants.length}){" "}
                <span className="muted" style={{ fontWeight: 400 }}>
                  行を選択して精算残高を確認・編集
                </span>
              </h2>
              <div className="table-wrap">
                <table>
                  <thead>
                    <tr>
                      <th>コード</th>
                      <th>名称</th>
                      <th className="num">手数料</th>
                      <th className="num">精算残高</th>
                      <th>状態</th>
                      <th>ID</th>
                      <th>作成日時</th>
                    </tr>
                  </thead>
                  <tbody>
                    {merchants.length === 0 ? (
                      <tr>
                        <td colSpan={7} className="empty">
                          加盟店がありません
                        </td>
                      </tr>
                    ) : (
                      merchants.map((m) => (
                        <tr
                          key={m.id}
                          className="clickable"
                          onClick={() => setSelected(m.id)}
                        >
                          <td className="mono">{m.code}</td>
                          <td>{m.name}</td>
                          <td className="num">{pct(m.fee_bps)}</td>
                          <td className={"num" + (m.collected < 0 ? " neg" : "")}>{yen(m.collected)}</td>
                          <td onClick={(e) => e.stopPropagation()}>
                            <select
                              value={m.status}
                              onChange={async (e) => {
                                try {
                                  await api.post(`/v1/admin/merchants/${m.id}/status`, {
                                    status: e.target.value,
                                  });
                                  toast("状態を更新しました");
                                  state.reload();
                                } catch (err) {
                                  toast(errMsg(err));
                                  state.reload();
                                }
                              }}
                            >
                              {STATUSES.map((s) => (
                                <option key={s} value={s}>
                                  {s}
                                </option>
                              ))}
                            </select>
                          </td>
                          <td className="mono muted">{m.id.slice(0, 8)}…</td>
                          <td className="muted">{fmtTime(m.created_at)}</td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
            </div>

            {selected && (
              <MerchantDetail
                key={selected}
                merchant={merchants.find((m) => m.id === selected) ?? null}
                merchantId={selected}
                onListChanged={state.reload}
                onRevealKey={setKeyReveal}
              />
            )}
          </>
        )}
      </Async>

      {keyReveal && <KeyModal reveal={keyReveal} onClose={() => setKeyReveal(null)} />}
    </>
  );
}

function CreateMerchant({
  onCreated,
}: {
  onCreated: (r: CreateMerchantResp, code: string) => void;
}) {
  const toast = useToast();
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [fee, setFee] = useState("0");
  const [credit, setCredit] = useState("0");
  const [busy, setBusy] = useState(false);

  const create = async () => {
    if (!code.trim() || !name.trim()) return toast("コードと名称を入力してください");
    setBusy(true);
    try {
      const r = await api.post<CreateMerchantResp>("/v1/merchants", {
        code: code.trim(),
        name: name.trim(),
        fee_bps: parseInt(fee, 10) || 0,
        credit_limit: parseInt(credit, 10) || 0,
      });
      setCode("");
      setName("");
      setFee("0");
      setCredit("0");
      onCreated(r, code.trim());
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <h2>新規加盟店</h2>
      <div className="row">
        <div className="field">
          <label>コード</label>
          <input value={code} onChange={(e) => setCode(e.target.value)} placeholder="shop-1" />
        </div>
        <div className="field">
          <label>名称</label>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="テスト店舗" />
        </div>
        <div className="field">
          <label>手数料(bps)</label>
          <input
            type="number"
            min={0}
            max={10000}
            value={fee}
            onChange={(e) => setFee(e.target.value)}
            style={{ width: 110 }}
          />
        </div>
        <div className="field">
          <label>与信限度(円)</label>
          <input
            type="number"
            min={0}
            value={credit}
            onChange={(e) => setCredit(e.target.value)}
            style={{ width: 130 }}
          />
        </div>
        <button className="primary" onClick={create} disabled={busy}>
          作成
        </button>
      </div>
    </div>
  );
}

function MerchantDetail({
  merchant,
  merchantId,
  onListChanged,
  onRevealKey,
}: {
  merchant: Merchant | null;
  merchantId: string;
  onListChanged: () => Promise<void>;
  onRevealKey: (r: KeyReveal) => void;
}) {
  const toast = useToast();
  const state = useAsync<AdminTxn[]>(
    () => api.get<AdminTxn[]>("/v1/admin/transactions" + qs({ limit: 20, merchant_id: merchantId })),
    [merchantId],
  );

  const [fee, setFee] = useState(String(merchant?.fee_bps ?? 0));
  const [credit, setCredit] = useState(String(merchant?.credit_limit ?? 0));
  const [adjSign, setAdjSign] = useState("1");
  const [adjAmount, setAdjAmount] = useState("");
  const [adjReason, setAdjReason] = useState("");

  const refresh = async () => {
    await onListChanged();
    await state.reload();
  };

  const setFeeBps = async () => {
    const fee_bps = parseInt(fee, 10);
    if (!(fee_bps >= 0 && fee_bps <= 10000)) return toast("0〜10000 bps の範囲で入力してください");
    try {
      await api.post(`/v1/admin/merchants/${merchantId}/fee`, { fee_bps });
      toast("手数料率を更新しました");
      refresh();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const setCreditLimit = async () => {
    const credit_limit = parseInt(credit, 10);
    if (!(credit_limit >= 0)) return toast("0 以上で入力してください");
    try {
      await api.post(`/v1/admin/merchants/${merchantId}/credit-limit`, { credit_limit });
      toast("与信限度を更新しました");
      refresh();
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const rotate = async () => {
    if (!confirm("この加盟店の既存 API キーをすべて失効し、新しいキーを発行します。よろしいですか?")) return;
    try {
      const r = await api.post<RotateKeyResp>(`/v1/admin/merchants/${merchantId}/api-keys`);
      onRevealKey({ title: "API キー(再発行)", secret: r.api_key, merchantId: r.merchant_id || merchantId });
    } catch (e) {
      toast(errMsg(e));
    }
  };
  const adjust = async () => {
    const amount = parseInt(adjAmount, 10);
    if (!amount || amount <= 0) return toast("金額を入力してください");
    try {
      const r = await api.post<MerchantAdjustResp>(`/v1/admin/merchants/${merchantId}/adjust`, {
        delta: parseInt(adjSign, 10) * amount,
        reason: adjReason.trim() || null,
      });
      toast(`調整完了: 精算残高 ${yen(r.balance)}`);
      setAdjAmount("");
      setAdjReason("");
      refresh();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const m = merchant;

  return (
    <div className="panel">
      <h2>
        加盟店詳細 <span className="mono" style={{ fontWeight: 400 }}>{m?.code}</span>
      </h2>
      {m && (
        <div className="cards" style={{ marginBottom: 16 }}>
          <div className="stat">
            <div className="label">精算残高</div>
            <div className={"value" + (m.collected < 0 ? " neg" : "")}>{yen(m.collected)}</div>
          </div>
          <div className="stat">
            <div className="label">決済手数料率</div>
            <div className="value">{pct(m.fee_bps)}</div>
          </div>
          <div className="stat">
            <div className="label">与信限度</div>
            <div className="value">{yen(m.credit_limit)}</div>
          </div>
          <div className="stat">
            <div className="label">チャージ余力</div>
            <div className={"value" + (m.collected + m.credit_limit < 0 ? " neg" : "")}>
              {yen(m.collected + m.credit_limit)}
            </div>
          </div>
        </div>
      )}

      <div className="row" style={{ marginBottom: 16 }}>
        <div className="field">
          <label>手数料率(bps, 1bps=0.01%)</label>
          <input
            type="number"
            min={0}
            max={10000}
            value={fee}
            onChange={(e) => setFee(e.target.value)}
            style={{ width: 150 }}
          />
        </div>
        <button onClick={setFeeBps}>手数料を更新</button>
      </div>
      <div className="row" style={{ marginBottom: 16 }}>
        <div className="field">
          <label>与信限度(円, topup 用のマイナス許容額)</label>
          <input
            type="number"
            min={0}
            value={credit}
            onChange={(e) => setCredit(e.target.value)}
            style={{ width: 170 }}
          />
        </div>
        <button onClick={setCreditLimit}>与信限度を更新</button>
      </div>
      <div style={{ marginBottom: 18 }}>
        <button onClick={rotate}>API キー再発行</button>
        <span className="muted" style={{ marginLeft: 8 }}>
          既存キーを失効し、新しいキーを発行します
        </span>
      </div>

      <h3 style={{ marginBottom: 8 }}>精算残高の調整(監査記録に残ります)</h3>
      <div className="row" style={{ marginBottom: 6 }}>
        <div className="field">
          <label>種別</label>
          <select value={adjSign} onChange={(e) => setAdjSign(e.target.value)}>
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
          <input value={adjReason} onChange={(e) => setAdjReason(e.target.value)} placeholder="調整理由(任意)" />
        </div>
        <button className="primary" onClick={adjust}>
          適用
        </button>
      </div>
      <p className="muted" style={{ margin: "0 0 18px" }}>
        精算残高 = 受領した支払 − チャージ取扱 − 返金・取消 + 調整(発行者が加盟店に支払う額)。
      </p>

      <h3 style={{ marginBottom: 8 }}>最近の取引</h3>
      <Async state={state}>
        {(txns) => (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>日時</th>
                  <th>種別</th>
                  <th>IDi</th>
                  <th className="num">金額</th>
                </tr>
              </thead>
              <tbody>
                {txns.length === 0 ? (
                  <tr>
                    <td colSpan={4} className="empty">
                      取引なし
                    </td>
                  </tr>
                ) : (
                  txns.map((t) => {
                    const sign =
                      t.kind === "payment" ? "pos" : t.kind === "refund" || t.kind === "reversal" ? "neg" : "";
                    return (
                      <tr key={t.id}>
                        <td className="muted">{fmtTime(t.occurred_at)}</td>
                        <td className="kind">{t.kind}</td>
                        <td className="mono">{t.idi}</td>
                        <td className={"num " + sign}>{yen(t.amount)}</td>
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
  );
}

function KeyModal({ reveal, onClose }: { reveal: KeyReveal; onClose: () => void }) {
  const toast = useToast();
  return (
    <Modal title={reveal.title} onClose={onClose}>
      <p className="muted">
        この API キーは<strong>今だけ</strong>表示されます。安全な場所に保存してください(サーバはハッシュのみ保持します)。
      </p>
      <div className="k">
        <input
          className="mono"
          style={{ flex: 1 }}
          readOnly
          value={reveal.secret}
          onClick={(e) => (e.target as HTMLInputElement).select()}
        />
        <button
          onClick={() => {
            navigator.clipboard?.writeText(reveal.secret);
            toast("コピーしました");
          }}
        >
          コピー
        </button>
      </div>
      <div className="muted mono" style={{ marginTop: 8 }}>
        merchant_id: {reveal.merchantId}
      </div>
      <div style={{ textAlign: "right", marginTop: 16 }}>
        <button className="primary" onClick={onClose}>
          閉じる
        </button>
      </div>
    </Modal>
  );
}
