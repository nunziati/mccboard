// Desktop profile notices (ticket #40): a profile the app reopened without mccboard, or that
// an update closed. The core computes them, with their texts, in the app's language.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type DesktopNoticeKind = 'reopened_without_mccboard' | 'closed_by_update'

export interface DesktopNotice {
  account: string
  label: string
  color: string
  kind: DesktopNoticeKind
  /** The copy reopened without mccboard still runs: it has to be closed before reopening. */
  stray_running: boolean
  since: string
  /** What happened and what to do, in the app's language. */
  message: string
  tooltip: string
  /** "Reopen B". */
  reopen: string
  can_reopen: boolean
}

export interface DesktopNotices {
  language: string
  notices: DesktopNotice[]
}

/** The notices as of the app's last look at the desktop app's processes. */
export function desktopNotices(): Promise<DesktopNotices> {
  return invoke<DesktopNotices>('desktop_notices')
}

/** Hides the account's notice until it happens again. */
export function dismissDesktopNotice(uuid: string): Promise<boolean> {
  return invoke<boolean>('dismiss_desktop_notice', { uuid })
}

/** Called with the notices whenever they change. */
export function onDesktopNotices(onChange: (notices: DesktopNotices) => void): Promise<UnlistenFn> {
  return listen<DesktopNotices>('desktop-notices', (event) => onChange(event.payload))
}
