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
  /** For `instance_in_use`: the Claude Code processes using the instance. */
  processes?: ClaudeProcess[] | null
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

// The doctor: drift of each instance against the sharing table, and unknown items to answer.

/** What is wrong with an item. Later checks add kinds; unknown ones show the core's text. */
export type Problem =
  | { kind: 'missing_link' }
  | { kind: 'unlinked_skill'; skill: string }
  | { kind: 'broken_link'; target: string | null }
  | { kind: 'wrong_target'; target: string }
  | { kind: 'link_chain'; target: string }
  | { kind: 'not_a_link' }
  | { kind: 'whole_folder_link'; target: string | null }
  | { kind: 'missing_launcher' }
  | { kind: 'stale_launcher'; found: string | null }
  /** A Global config key of `.claude.json` differs: the primary's value and this instance's (null when unset). */
  | { kind: 'misaligned_key'; key: string; primary: unknown; instance: unknown | null }
  | { kind: 'missing_marketplace'; marketplace: string }
  | { kind: 'missing_plugin'; plugin: string }

/** What a repair would do. */
export type Repair =
  | { action: 'create_link'; target: string }
  | { action: 'replace_link'; target: string }
  | { action: 'backup_and_link'; target: string }
  | { action: 'migrate_skills'; skills: string[] }
  | { action: 'migrate_plugins' }
  | { action: 'sync_launchers' }
  | { action: 'align_key'; key: string; value: unknown }
  /** `source` is null when mccboard can't tell how to add the marketplace. */
  | { action: 'add_marketplace'; marketplace: string; source: string | null }
  | { action: 'install_plugin'; plugin: string }

export interface DriftItem {
  /** Stable across runs: `<family>:<subject>`, e.g. `link:settings.json`. */
  id: string
  item: string
  path: string
  problem: Problem
  repair: Repair
  /** The core's English wording, used when the UI has no translation for the kind. */
  explanation: string
  repair_explanation: string
}

export interface InstanceDrift {
  config_dir: string
  kind: InstanceKind
  items: DriftItem[]
  drift_count: number
}

export interface SharingQuestion {
  item: string
  path: string
  is_dir: boolean
}

export interface Doctor {
  instances: InstanceDrift[]
  questions: SharingQuestion[]
  drift_count: number
}

export type SharingAnswer = 'link' | 'private'

export function doctor(): Promise<Doctor> {
  return invoke<Doctor>('doctor')
}

export function answerSharing(item: string, answer: SharingAnswer): Promise<void> {
  return invoke<void>('answer_sharing', { item, answer })
}

// Repair: fixing an instance's drift, behind the session guard.

/** A running Claude Code process, as the session guard sees it. */
export interface ClaudeProcess {
  pid: number
  name: string
  config_dir: { kind: 'default' } | { kind: 'dir'; path: string } | { kind: 'unknown' }
  cwd: string | null
}

export type RepairOutcome =
  | { outcome: 'done'; backup: string | null }
  | { outcome: 'failed'; reason: string }

export interface RepairedItem {
  id: string
  item: string
  repair: Repair
  outcome: RepairOutcome
}

export interface RepairReport {
  config_dir: string
  items: RepairedItem[]
  /** The instance's drift after the repair. */
  drift: InstanceDrift
}

/** Repairs the instance in `path`: every drift item, or only the one with id `item`. */
export function repair(path: string, item?: string): Promise<RepairReport> {
  return invoke<RepairReport>('repair', { path, item: item ?? null })
}

// Removing an instance: from mccboard only, or with its folder too.

export type FolderOutcome = 'kept' | 'deleted' | 'already_gone'

export interface Removal {
  config_dir: string
  folder: FolderOutcome
}

/**
 * Removes the linked instance in `path` and its launcher. With `deleteFolder`, which the user
 * must have confirmed, also deletes its folder; refused while Claude Code runs there.
 */
export function removeInstance(path: string, deleteFolder: boolean): Promise<Removal> {
  return invoke<Removal>('remove_instance', { path, deleteFolder })
}

// Adding a linked instance, with login, and logging an instance in on its own.

/** Whether the login ran: only the account observed afterwards says whether it worked. */
export type LoginRun =
  | { run: 'ran'; exit_code: number | null }
  | { run: 'already_logged_in' }
  | { run: 'not_requested' }

export type LoginWarning = {
  kind: 'account_has_other_instances'
  account: string
  label: string | null
  instances: string[]
}

export interface LoginReport {
  config_dir: string
  run: LoginRun
  /** The instance as registered now. */
  instance: InstanceStatus
  /** The account it is logged in with; null when none can be observed. */
  account: Account | null
  new_account: boolean
  warnings: LoginWarning[]
}

export interface AddReport {
  config_dir: string
  created: boolean
  /** The links and alignment made, as a repair of the new instance. */
  setup: RepairReport
  /** Unknown items of the primary instance still unanswered: not linked. */
  sharing_questions: SharingQuestion[]
  login: LoginReport
}

/** The folder a linked instance for `label` gets by default (`~/.claude-<label>`). */
export function addFolder(label: string): Promise<string> {
  return invoke<string>('add_folder', { label })
}

/**
 * Adds a linked instance for the account labelled `label`, in `path` or the default folder,
 * and opens Claude Code's login. Resolves once the login has ended.
 */
export function addInstance(label: string, path: string | null): Promise<AddReport> {
  return invoke<AddReport>('add_instance', { label, path })
}

/** Opens Claude Code's login in the registered instance in `path`; resolves once it ended. */
export function loginInstance(path: string): Promise<LoginReport> {
  return invoke<LoginReport>('login_instance', { path })
}
