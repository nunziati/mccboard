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

/** Why Claude Code didn't leave a valid token when asked to renew it. */
export type RenewalFailure =
  | { kind: 'claude_not_found' }
  | { kind: 'timed_out' }
  | { kind: 'exited'; code: number | null }
  | { kind: 'still_expired' }
  | { kind: 'not_started' }

/** How far the reading can be trusted as current. */
export type Freshness = 'current' | 'stale' | 'no_reading'

/** Why an account's usage isn't being refreshed. */
export type UsageBlocker =
  | { kind: 'token_expired' }
  | { kind: 'not_logged_in' }
  | { kind: 'credentials_unreadable' }
  | { kind: 'refresh_failed'; failure: UsageFailure }

/** Where the token renewal of an account stands, as the core decides it (#30, #53). */
export type RenewalSituation =
  | { state: 'renewed'; config_dir: string; at: string }
  | {
      state: 'failed'
      config_dir: string
      at: string
      failure: RenewalFailure
      /** Null when the next refresh may try again. */
      retry_at: string | null
    }
  /** Null `next_at`: the next refresh may try. */
  | { state: 'waiting'; config_dir: string; next_at: string | null }
  | { state: 'off'; config_dir: string }
  | { state: 'quiet' }

/** Where an account's usage stands, as of the report. */
export interface UsageSituation {
  reading: UsageReading | null
  freshness: Freshness
  blocker: UsageBlocker | null
  renewal: RenewalSituation
}

export interface AccountUsage {
  account: Account
  called: boolean
  next_call_at: string | null
  situation: UsageSituation
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

/** The taskbar widget's preferences (config.toml), and whether this platform has it (Windows). */
export interface TaskbarWidgetSettings {
  supported: boolean
  /** "Show the widget on the taskbar". */
  on: boolean
  /** "Show account labels" in it. */
  labels: boolean
}

export function taskbarWidget(): Promise<TaskbarWidgetSettings> {
  return invoke<TaskbarWidgetSettings>('taskbar_widget')
}

/** Saves the taskbar widget's preferences given; the others stay. */
export function setTaskbarWidget(change: { on?: boolean; labels?: boolean }): Promise<void> {
  return invoke<void>('set_taskbar_widget', { on: change.on ?? null, labels: change.labels ?? null })
}
