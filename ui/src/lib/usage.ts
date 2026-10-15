// Usage readings, mirroring the core's usage types through the Tauri binding. The tray (#8)
// and the windows render them; the UI computes nothing of its own.

import { invoke } from '@tauri-apps/api/core'
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
  | { status: 'login_expired' }
  | { status: 'not_logged_in' }
  | { status: 'credentials_unreadable' }
  | { status: 'failed'; failure: UsageFailure }

export interface AccountUsage {
  account: Account
  status: UsageStatus
  reading: UsageReading | null
  stale: boolean
  called: boolean
  next_call_at: string | null
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
