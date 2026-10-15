// Types and calls mirroring the core's public interface, through the Tauri binding.
// The UI holds no logic of its own: it renders what the core returns.

import { invoke } from '@tauri-apps/api/core'

export type InstanceKind = 'primary' | 'linked'

export type LoginState =
  | { state: 'logged_in'; expires_at: string }
  | { state: 'expired'; expired_at: string }
  | { state: 'not_logged_in' }
  | { state: 'unknown' }

export type Warning = 'global_config_unreadable' | 'credentials_unreadable'

export interface ObservedAccount {
  uuid: string
  email: string | null
}

export interface Candidate {
  config_dir: string
  kind: InstanceKind
  account: ObservedAccount | null
  login: LoginState
  warnings: Warning[]
}

/** A typed core error: `code` is stable and translated by the UI. */
export interface CoreError {
  code: string
}

export function isCoreError(value: unknown): value is CoreError {
  return typeof value === 'object' && value !== null && typeof (value as CoreError).code === 'string'
}

export function scan(): Promise<Candidate[]> {
  return invoke<Candidate[]>('scan')
}
