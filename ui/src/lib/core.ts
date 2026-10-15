// Types and calls mirroring the core's public interface, through the Tauri binding.
// The UI holds no logic of its own: it renders what the core returns.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type InstanceKind = 'primary' | 'linked'

export type LoginState =
  | { state: 'logged_in'; expires_at: string }
  | { state: 'expired'; expired_at: string }
  | { state: 'not_logged_in' }
  | { state: 'unknown' }

export type Warning = 'global_config_unreadable' | 'credentials_unreadable' | 'config_dir_missing'

export interface ObservedAccount {
  uuid: string
  email: string | null
}

/** What an instance looks like right now. */
export interface Observation {
  account: ObservedAccount | null
  login: LoginState
  warnings: Warning[]
}

export interface Candidate extends Observation {
  config_dir: string
  kind: InstanceKind
  registered: boolean
}

export interface Account {
  uuid: string
  label: string
  color: string
  email: string | null
  launcher: string | null
  launcher_name: string
}

export type AccountStatus =
  | { status: 'unchanged' }
  | { status: 'changed'; registered: string }
  | { status: 'unrecorded' }
  | { status: 'not_observed' }

export interface InstanceStatus extends Observation {
  config_dir: string
  kind: InstanceKind
  registered_account: string | null
  account_status: AccountStatus
}

export interface Registry {
  accounts: Account[]
  instances: InstanceStatus[]
}

export interface Registration {
  instance: InstanceStatus
  new_account: Account | null
}

/** Changes to an account. For `launcher`, an empty string drops the override. */
export interface AccountUpdate {
  label?: string
  color?: string
  launcher?: string
}

/** A launcher as found: its command name and the config directory it starts Claude Code in. */
export interface Launcher {
  name: string
  config_dir: string | null
}

/** Where launchers live: a block of the shell rc file (Linux) or mccboard's bin folder (Windows). */
export type LauncherPlace =
  | { kind: 'rc_file'; path: string }
  | { kind: 'bin_folder'; path: string; on_user_path: boolean }

export type LauncherState =
  | { state: 'ok' }
  | { state: 'missing' }
  | { state: 'stale'; found: string | null }

/** A registered linked instance's launcher. */
export type InstanceLauncher = { config_dir: string; name: string } & LauncherState

export interface LauncherStatus {
  place: LauncherPlace
  instances: InstanceLauncher[]
  orphans: Launcher[]
}

export type UserPathChange =
  | { status: 'already_on' }
  | { status: 'added' }
  | { status: 'not_added'; reason: string }

export interface LauncherSync {
  created: Launcher[]
  updated: Launcher[]
  removed: Launcher[]
  user_path: UserPathChange | null
}

/** A typed core error: `code` is stable and translated by the UI. */
export interface CoreError {
  code: string
  path?: string | null
  detail?: string | null
}

export function isCoreError(value: unknown): value is CoreError {
  return typeof value === 'object' && value !== null && typeof (value as CoreError).code === 'string'
}

export function scan(): Promise<Candidate[]> {
  return invoke<Candidate[]>('scan')
}

export function registry(): Promise<Registry> {
  return invoke<Registry>('registry')
}

export function register(path: string): Promise<Registration> {
  return invoke<Registration>('register', { path })
}

export function unregister(path: string): Promise<void> {
  return invoke<void>('unregister', { path })
}

export function updateAccount(uuid: string, update: AccountUpdate): Promise<Account> {
  return invoke<Account>('update_account', { uuid, ...update })
}

export function launchers(): Promise<LauncherStatus> {
  return invoke<LauncherStatus>('launchers')
}

export function syncLaunchers(): Promise<LauncherSync> {
  return invoke<LauncherSync>('sync_launchers')
}

/** Calls `onChange` whenever config.toml changes on disk (CLI, editor, this app). */
export function onConfigChanged(onChange: () => void): Promise<UnlistenFn> {
  return listen('config-changed', onChange)
}
