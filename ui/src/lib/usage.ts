// Usage readings, mirroring the core's usage types through the Tauri binding. The tray (#8)
// and the windows render them; the UI computes nothing of its own.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { Account } from './core'

/** A usage window: utilization 0–100; `resets_at` is null once the reset time has passed. */
export interface UsageWindow {
  utilization: number
  resets_at: string | null
}

/** A 7-day window for one model, named by the endpoint. */
export interface ModelWindow extends UsageWindow {
  model: string
}

export interface UsageReading {
  source: 'endpoint'
  read_at: string
  session: UsageWindow | null
  weekly: UsageWindow | null
  models: ModelWindow[]
}

export type UsageFailure =
  | { kind: 'rate_limited' }
  | { kind: 'unauthorized' }
  | { kind: 'forbidden' }
  | { kind: 'http_status'; status: number }
  | { kind: 'unreachable'; reason: string }
  | { kind: 'malformed' }

export type UsageStatus =
  | { status: 'ok' }
  | { status: 'never_read' }
  | { status: 'token_expired' }
  | { status: 'not_logged_in' }
  | { status: 'credentials_unreadable' }
  | { status: 'failed'; failure: UsageFailure }

/** Why Claude Code didn't leave a valid token when asked to renew it. */
export type RenewalFailure =
  | { kind: 'claude_not_found' }
  | { kind: 'timed_out' }
  | { kind: 'exited'; code: number | null }
  | { kind: 'still_expired' }
  | { kind: 'not_started' }

export type RenewalAttempt = { at: string } & (
  | { outcome: 'renewed' }
  | { outcome: 'failed'; failure: RenewalFailure }
)

/** Claude Code renewing an account's expired token in one of its instances (#30). */
export interface TokenRenewal {
  config_dir: string
  attempted: boolean
  last: RenewalAttempt | null
  /** While the token is expired: no attempt before then; null when the next refresh may try. */
  next_attempt_at: string | null
}

export interface AccountUsage {
  account: Account
  status: UsageStatus
  reading: UsageReading | null
  stale: boolean
  called: boolean
  next_call_at: string | null
  /** Null when the preference is off, or when no renewal was ever needed. */
  renewal: TokenRenewal | null
}

export type UsageWarning = 'state_discarded' | 'state_unwritable'

export interface UsageReport {
  accounts: AccountUsage[]
  warnings: UsageWarning[]
}

/** The last known readings, without calling the endpoint. */
export function usage(): Promise<UsageReport> {
  return invoke<UsageReport>('usage')
}

/** Refreshes usage within the rate limits (a manual refresh too), then reports it. */
export function refreshUsage(): Promise<UsageReport> {
  return invoke<UsageReport>('refresh_usage')
}

/** Calls `onChange` after each of the tray's usage refreshes, which may renew a token. */
export function onUsageChanged(onChange: () => void): Promise<UnlistenFn> {
  return listen('usage-changed', onChange)
}

/** Whether a usage refresh may ask Claude Code to renew an expired token. */
export function renewExpiredTokens(): Promise<boolean> {
  return invoke<boolean>('renew_expired_tokens')
}

/** Saves whether a usage refresh may ask Claude Code to renew an expired token. */
export function setRenewExpiredTokens(on: boolean): Promise<void> {
  return invoke<void>('set_renew_expired_tokens', { on })
}
