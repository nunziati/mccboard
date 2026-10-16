<script lang="ts">
  // The "Desktop profiles" section: the core's desktop profiles, the default one first, with
  // Add, Open, Remove and "open when mccboard starts", and the first sign-in guide. It reloads
  // when config.toml changes and, while the window is visible, every few seconds, so the
  // running state and a new sign-in show up.
  import { onMount } from 'svelte'
  import AddDesktopProfile from './AddDesktopProfile.svelte'
  import RemoveDesktopProfile from './RemoveDesktopProfile.svelte'
  import SignInGuide from './SignInGuide.svelte'
  import {
    addDesktopProfile,
    desktopProfileCandidates,
    desktopProfiles,
    isCoreError,
    onConfigChanged,
    openDesktopProfile,
    removeDesktopProfile,
    setDesktopProfileOpenAtStart,
    type Account,
    type CoreError,
    type DesktopProfile,
    type DesktopWarning,
  } from './core'
  import { errorMessage, t } from './i18n.svelte'

  let {
    accounts,
    busy,
    onaction,
    defaultAccount = $bindable(null),
  }: {
    /** The registered accounts, to name the accounts a profile is signed in with. */
    accounts: Account[]
    busy: boolean
    /** Runs an action the way the window does: busy, notice or error, then a reload. */
    onaction: (action: () => Promise<string | null>) => Promise<void>
    /** Out: the default desktop profile's account, whose color can't be edited. */
    defaultAccount?: string | null
  } = $props()

  /** How often to reload while a sign-in is awaited, and otherwise. */
  const FAST_MS = 3000
  const SLOW_MS = 10000

  let profiles = $state<DesktopProfile[] | null>(null)
  let candidates = $state<Account[]>([])
  let loadError = $state<CoreError | null>(null)
  /** Profiles opened from this window, by account: their guide shows before the app is seen running. */
  let opened = $state<Record<string, boolean>>({})
  /** Profiles whose guide the user dismissed, by account, until opened again. */
  let dismissed = $state<Record<string, boolean>>({})

  /** The profile awaiting its first sign-in: opened or running, with no account observed yet. */
  let guide = $derived(
    profiles?.find(
      (p) =>
        !p.default &&
        p.account !== null &&
        p.observed.account === null &&
        !dismissed[p.account] &&
        (p.running || opened[p.account]),
    ) ?? null,
  )

  let awaitingSignIn = $derived(guide !== null)

  async function load() {
    try {
      const [next, nextCandidates] = await Promise.all([desktopProfiles(), desktopProfileCandidates()])
      profiles = next
      candidates = nextCandidates
      defaultAccount = next.find((p) => p.default)?.account ?? null
      loadError = null
    } catch (e) {
      loadError = isCoreError(e) ? e : { code: 'unknown' }
    }
  }

  async function act(action: () => Promise<string | null>) {
    await onaction(action)
    await load()
  }

  function nameOf(profile: DesktopProfile): string {
    if (profile.label) return profile.label
    return profile.default ? t('desktop.default_profile') : (profile.account ?? '')
  }

  function accountName(uuid: string, label: string | null = null): string {
    const account = accounts.find((a) => a.uuid === uuid)
    const name = label ?? account?.label
    const email = account?.email
    if (name && email) return `${name} (${email})`
    return name ?? email ?? uuid
  }

  function warningText(profile: DesktopProfile, warning: DesktopWarning): string {
    switch (warning.kind) {
      case 'observed_account_differs':
        return t('desktop.warning.observed_account_differs', {
          observed: accountName(warning.observed, warning.observed_label),
          name: nameOf(profile),
        })
      case 'default_profile_not_primary_account':
        return t('desktop.warning.default_profile_not_primary_account', {
          observed: accountName(warning.observed, warning.observed_label),
        })
      case 'no_local_instance':
        return t('desktop.warning.no_local_instance')
    }
  }

  function add(uuid: string, folder: string | null): Promise<boolean> {
    let added = false
    return act(async () => {
      const profile = await addDesktopProfile(uuid, folder)
      added = true
      return t('desktop.notice.added', { name: nameOf(profile), path: profile.data_folder })
    }).then(() => added)
  }

  function open(profile: DesktopProfile) {
    return act(async () => {
      await openDesktopProfile(profile.default ? null : profile.account)
      if (!profile.default && profile.account && profile.observed.account === null) {
        opened[profile.account] = true
        dismissed[profile.account] = false
      }
      return t('desktop.notice.opened', { name: nameOf(profile) })
    })
  }

  function setOpenAtStart(profile: DesktopProfile, on: boolean) {
    const account = profile.account
    if (!account) return
    return act(async () => {
      await setDesktopProfileOpenAtStart(account, on)
      return null
    })
  }

  function remove(profile: DesktopProfile, deleteFolder: boolean) {
    const account = profile.account
    if (!account) return
    return act(async () => {
      const removal = await removeDesktopProfile(account, deleteFolder)
      const key =
        removal.folder === 'deleted'
          ? 'desktop.notice.removed_deleted'
          : removal.folder === 'already_gone'
            ? 'desktop.notice.removed_already_gone'
            : 'desktop.notice.removed'
      return t(key, { name: nameOf(profile), path: removal.data_folder })
    })
  }

  // Reloads while the window is visible: quickly while a sign-in is awaited.
  $effect(() => {
    const every = awaitingSignIn ? FAST_MS : SLOW_MS
    const timer = setInterval(() => {
      if (!document.hidden) load()
    }, every)
    return () => clearInterval(timer)
  })

  onMount(() => {
    load()
    const unlisten = onConfigChanged(load)
    return () => {
      unlisten.then((stop) => stop())
    }
  })
</script>

<section>
  <h2>{t('desktop.title')}</h2>
  <p class="muted small">{t('desktop.hint')}</p>

  {#if loadError}
    <p class="error" role="alert">{errorMessage(loadError.code)}</p>
  {/if}

  {#if guide}
    <SignInGuide
      name={nameOf(guide)}
      ondismiss={() => {
        if (guide?.account) dismissed[guide.account] = true
      }}
    />
  {/if}

  {#if profiles}
    <table>
      <thead>
        <tr>
          <th>{t('desktop.column.profile')}</th>
          <th>{t('desktop.column.folder')}</th>
          <th>{t('desktop.column.state')}</th>
          <th>{t('desktop.column.signed_in')}</th>
          <th>{t('desktop.column.open_at_start')}</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each profiles as profile (profile.default ? 'default' : profile.account)}
          <tr>
            <td>
              <div class="profile">
                <span class="swatch" style:background={profile.color}></span>
                <strong>{nameOf(profile)}</strong>
                {#if profile.default && profile.label}
                  <span class="muted small">{t('desktop.default')}</span>
                {/if}
              </div>
              {#if profile.default}
                <div class="muted small">{t('desktop.default_hint')}</div>
              {/if}
            </td>
            <td><code>{profile.data_folder}</code></td>
            <td>
              {#if profile.running}
                <span class="ok">{t('desktop.running')}</span>
              {:else}
                <span class="muted">{t('desktop.not_running')}</span>
              {/if}
            </td>
            <td>
              {#if profile.observed.account}
                <div>{accountName(profile.observed.account)}</div>
              {:else}
                <span class="muted">{t('desktop.not_signed_in')}</span>
              {/if}
              {#if profile.observed.others.length > 0}
                <div class="muted small">
                  {t('desktop.signed_in_before', {
                    names: profile.observed.others.map((uuid) => accountName(uuid)).join(', '),
                  })}
                </div>
              {/if}
              {#each profile.warnings as warning (warning.kind)}
                <div class="warning small">{warningText(profile, warning)}</div>
              {/each}
            </td>
            <td>
              {#if profile.default}
                <span class="muted">—</span>
              {:else}
                <input
                  type="checkbox"
                  title={t('desktop.open_at_start_hint')}
                  aria-label={t('desktop.open_at_start_hint')}
                  checked={profile.open_at_start}
                  disabled={busy}
                  onchange={(e) => setOpenAtStart(profile, (e.currentTarget as HTMLInputElement).checked)}
                />
              {/if}
            </td>
            <td class="row-actions">
              <button disabled={busy} title={t('desktop.open_hint')} onclick={() => open(profile)}>
                {t('action.open')}
              </button>
              {#if !profile.default}
                <RemoveDesktopProfile
                  name={nameOf(profile)}
                  folder={profile.data_folder}
                  {busy}
                  onremove={(deleteFolder) => remove(profile, deleteFolder)}
                />
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>

    <AddDesktopProfile {candidates} {busy} onadd={add} />
  {/if}
</section>

<style>
  h2 {
    margin: 1.25rem 0 0.5rem;
    font-size: 1.1rem;
  }
  p {
    margin: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 0.4rem;
  }
  th,
  td {
    text-align: left;
    vertical-align: top;
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid var(--border);
  }
  th {
    font-weight: 600;
    color: var(--muted);
  }
  .profile {
    display: flex;
    gap: 0.45rem;
    align-items: center;
  }
  .swatch {
    width: 0.9rem;
    height: 0.9rem;
    border-radius: 0.2rem;
    display: inline-block;
    flex-shrink: 0;
  }
  .row-actions {
    text-align: right;
    white-space: nowrap;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
  .ok {
    color: var(--ok);
  }
  .warning {
    color: var(--warn);
  }
  .error {
    color: var(--error);
    margin: 0.4rem 0;
  }
</style>
