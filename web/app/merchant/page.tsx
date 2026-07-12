"use client";

import { api } from "@/lib/api";
import type { Merchant } from "@/lib/types";
import { fmtTime, pct, yen } from "@/lib/format";
import { Async, useAsync } from "@/components/ui";

export default function MerchantOverview() {
  const state = useAsync<Merchant>(() => api.get<Merchant>("/v1/me"));

  return (
    <Async state={state}>
      {(me) => (
        <>
          <div className="cards">
            <div className="stat">
              <div className="label">精算残高(発行者からの受取額)</div>
              <div className={"value" + (me.collected < 0 ? " neg" : "")}>{yen(me.collected)}</div>
            </div>
            <div className="stat">
              <div className="label">決済手数料率</div>
              <div className="value">{pct(me.fee_bps)}</div>
            </div>
            <div className="stat">
              <div className="label">与信限度</div>
              <div className="value">{yen(me.credit_limit)}</div>
            </div>
            <div className="stat">
              <div className="label">チャージ可能額(余力)</div>
              <div className={"value" + (me.collected + me.credit_limit < 0 ? " neg" : "")}>
                {yen(me.collected + me.credit_limit)}
              </div>
            </div>
          </div>

          <div className="panel">
            <h2>加盟店情報</h2>
            <dl>
              <dt>コード</dt>
              <dd className="mono">{me.code}</dd>
              <dt>名称</dt>
              <dd>{me.name}</dd>
              <dt>状態</dt>
              <dd>
                <span className={"pill " + me.status}>{me.status}</span>
              </dd>
              <dt>加盟店 ID</dt>
              <dd className="mono muted">{me.id}</dd>
              <dt>登録日時</dt>
              <dd className="muted">{fmtTime(me.created_at)}</dd>
            </dl>
            <p className="muted" style={{ margin: "14px 0 0" }}>
              精算残高 = 受領した支払 − チャージ取扱 − 返金・取消 +
              調整。マイナスは発行者への支払い(集金した金額)を表します。
            </p>
          </div>
        </>
      )}
    </Async>
  );
}
