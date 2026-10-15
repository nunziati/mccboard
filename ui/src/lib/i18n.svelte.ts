// The i18n layer: every UI string goes through `t`. English is the source of truth for keys;
// a key missing from another language fails type-checking.

import en from './locales/en.json'
import it from './locales/it.json'

export type Key = keyof typeof en

const it_: Record<Key, string> = it
const dictionaries: Record<Lang, Record<Key, string>> = { en, it: it_ }

export const LANGUAGES = ['en', 'it'] as const
export type Lang = (typeof LANGUAGES)[number]

export const LANGUAGE_NAMES: Record<Lang, string> = { en: 'English', it: 'Italiano' }

function initialLanguage(): Lang {
  const preferred = typeof navigator === 'undefined' ? '' : navigator.language
  return preferred.toLowerCase().startsWith('it') ? 'it' : 'en'
}

let current = $state<Lang>(initialLanguage())

export const i18n = {
  get lang(): Lang {
    return current
  },
  set lang(lang: Lang) {
    current = lang
    document.documentElement.lang = lang
  },
}

/** The string for `key` in the current language, with `{name}` placeholders filled from `vars`. */
export function t(key: Key, vars: Record<string, string | number> = {}): string {
  const text = dictionaries[current][key] ?? en[key] ?? key
  return text.replace(/\{(\w+)\}/g, (_, name: string) => String(vars[name] ?? `{${name}}`))
}

/** A date and time in the current language's format and the user's time zone. */
export function formatTime(iso: string): string {
  return new Intl.DateTimeFormat(current, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(iso))
}

/** The translated message for a core error code. */
export function errorMessage(code: string): string {
  const key = `error.${code}`
  return key in en ? t(key as Key) : t('error.unknown')
}
