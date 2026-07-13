"use client";

import { useState } from "react";
import { api } from "@/lib/api";
import type {
  Health,
  IssuerBalance,
  Merchant,
  OutstandingReport,
  SweepResp,
} from "@/lib/types";
import { yen } from "@/lib/format";
import { Async, useAsync, errMsg } from "@/components/ui";
import { useToast } from "@/components/toast";

interface Overview {
  report: OutstandingReport;
  merchants: Merchant[];
  issuer: IssuerBalance;
  health: Health;
}

export default function AdminOverview() {
  const state = useAsync<Overview>(async () => {
    const [report, merchants, issuer, health] = await Promise.all([
      api.get<OutstandingReport>("/v1/admin/reports/outstanding-balance"),
      api.get<Merchant[]>("/v1/merchants"),
      api.get<IssuerBalance>("/v1/admin/issuer/balance"),
      api
        .get<Health>("/healthz")
        .catch(() => ({ status: "?", live_sessions: 0 }) as Health),
    ]);
    return { report, merchants, issuer, health };
  });

  return (
    <Async state={state}>
      {({ report, merchants, issuer, health }) => (
        <>
          <div className="cards">
            <Stat label="未使用残高(合計)" value={yen(report.total)} />
            <Stat
              label="発行者残高(収益)"
              value={yen(issuer.balance)}
              neg={issuer.balance < 0}
            />
            <Stat
              label="口座数"
              value={report.account_count.toLocaleString()}
            />
            <Stat label="加盟店数" value={merchants.length.toLocaleString()} />
            <Stat
              label="ライブセッション"
              value={(health.live_sessions ?? 0).toLocaleString()}
            />
          </div>
          <SweepPanel />
        </>
      )}
    </Async>
  );
}

function Stat({
  label,
  value,
  neg,
}: {
  label: string;
  value: string;
  neg?: boolean;
}) {
  return (
    <div className="stat">
      <div className="label">{label}</div>
      <div className={"value" + (neg ? " neg" : "")}>{value}</div>
    </div>
  );
}

function SweepPanel() {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");

  const run = async () => {
    setBusy(true);
    try {
      const r = await api.post<SweepResp>("/v1/admin/expiry/sweep");
      setResult(
        r.ran
          ? `完了: ${r.expired_buckets} バケット / ${yen(r.expired_amount)} を失効`
          : "他インスタンスが実行中のためスキップ",
      );
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel">
      <h2>失効スイープ(資金決済法)</h2>
      <p className="muted">
        有効期限を過ぎた残高を失効させ、台帳に記帳します。通常は自動実行されますが、手動でも実行できます。
      </p>
      <button className="primary" onClick={run} disabled={busy}>
        スイープを実行
      </button>
      <span className="muted" style={{ marginLeft: 12 }}>
        {result}
      </span>
    </div>
  );
}
