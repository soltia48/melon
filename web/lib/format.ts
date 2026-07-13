export const yen = (n: number | string | null | undefined): string =>
  "¥" + Number(n ?? 0).toLocaleString("ja-JP");

/** Signed yen, always showing the sign (used for ledger deltas). */
export const yenSigned = (n: number): string =>
  (n < 0 ? "−" : "") + "¥" + Math.abs(Number(n ?? 0)).toLocaleString("ja-JP");

/** A FeliCa system code as 0xNNNN. */
export const scHex = (n: number): string =>
  "0x" + Number(n).toString(16).toUpperCase().padStart(4, "0");

export const fmtTime = (s: string | null | undefined): string => {
  if (!s) return "";
  try {
    return new Date(s).toLocaleString("ja-JP");
  } catch {
    return String(s);
  }
};

/** Shorten a long id for compact display (keeps head and tail). */
export const shortId = (id: string | null | undefined): string => {
  const s = String(id ?? "");
  return s.length > 13 ? s.slice(0, 8) + "…" + s.slice(-4) : s;
};

export const pct = (bps: number): string =>
  (Number(bps) / 100).toFixed(2) + "%";
