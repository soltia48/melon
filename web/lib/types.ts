// Shapes returned by the melon-server JSON API. These mirror the Rust response
// structs in crates/melon-server/src/handlers.rs.

export type Role = "admin" | "merchant";

export interface User {
  id: string;
  email: string;
  name: string;
  role: Role;
  merchant_id: string | null;
  /** Merchant user's store scope: null = merchant-wide admin (all stores). */
  store_id: string | null;
  status: string;
  created_at: string;
}

/** A store (店舗) under a merchant. */
export interface Store {
  id: string;
  merchant_id: string;
  code: string;
  name: string;
  status: string;
  is_default: boolean;
  created_at: string;
}

/** A store-scoped API key (metadata only; the secret is shown once at issuance). */
export interface ApiKey {
  id: string;
  store_id: string | null;
  label: string | null;
  created_at: string;
  revoked_at: string | null;
  active: boolean;
}

export interface CreateApiKeyResp {
  id: string;
  store_id: string;
  api_key: string;
}

export interface LoginResp {
  user: User;
}

/** Unauthenticated sign-in page config (`GET /v1/auth/config`). */
export interface AuthConfig {
  /** Cloudflare Turnstile site key, or null when the challenge is disabled. */
  turnstile_site_key: string | null;
}

export interface Merchant {
  id: string;
  code: string;
  name: string;
  status: string;
  fee_bps: number;
  credit_limit: number;
  collected: number;
  created_at: string;
}

export interface CreateMerchantResp {
  merchant_id: string;
  api_key: string;
}

export interface RotateKeyResp {
  merchant_id: string;
  api_key: string;
}

export interface MerchantAdjustResp {
  id: string;
  delta: number;
  balance: number;
}

/** Admin account listing row. */
export interface AccountRow {
  system_code: number;
  idm: string;
  idi: string;
  status: string;
  balance: number;
  created_at: string;
}

export interface Bucket {
  bucket_id: string;
  remaining: number;
  expires_at: string;
}

export interface AdminBalance {
  system_code: number;
  idm: string;
  idi: string;
  total: number;
  buckets: Bucket[];
}

export interface AdjustResp {
  transaction_id: string;
  delta: number;
  balance: number;
  bucket_id: string | null;
}

/** Admin transaction row (raw card identity). */
export interface AdminTxn {
  id: string;
  system_code: number;
  idm: string;
  idi: string;
  kind: string;
  merchant_id: string | null;
  store_id: string | null;
  store_name: string | null;
  amount: number;
  fee: number;
  note: string | null;
  related_txn_id: string | null;
  occurred_at: string;
}

/** Merchant-facing transaction row (pseudonymous account id only). */
export interface MerchantTxn {
  id: string;
  account_id: string;
  kind: string;
  merchant_id: string | null;
  store_id: string | null;
  store_name: string | null;
  amount: number;
  fee: number;
  note: string | null;
  related_txn_id: string | null;
  occurred_at: string;
}

/** Admin refundable payment (raw card identity). */
export interface AdminRefundable {
  id: string;
  system_code: number;
  idm: string;
  idi: string;
  merchant_id: string | null;
  amount: number;
  fee: number;
  refunded: number;
  refundable: number;
  occurred_at: string;
}

export interface RefundResp {
  transaction_id: string;
  payment_txn_id: string;
  amount: number;
  balance: number;
  replayed: boolean;
}

export interface IssuerBalance {
  fee_income: number;
  expiry_income: number;
  adjustments: number;
  balance: number;
}

export interface IssuerAdjustment {
  id: string;
  amount: number;
  note: string | null;
  created_at: string;
}

export interface IssuerAdjustResp {
  id: string;
  delta: number;
  balance: number;
}

export interface ExpiryMonth {
  month: string;
  amount: number;
}

export interface OutstandingReport {
  as_of: string;
  total: number;
  account_count: number;
  by_expiry_month: ExpiryMonth[];
}

export interface SweepResp {
  ran: boolean;
  expired_buckets: number;
  expired_amount: number;
}

export interface Health {
  status: string;
  live_sessions: number;
}
