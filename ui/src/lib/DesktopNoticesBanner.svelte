<script lang="ts">
  // A banner per desktop profile the app reopened without mccboard or an update closed, with
  // "Reopen B" (the usual opening: it closes nothing) and Dismiss. Texts come from the core.
  import { onMount } from 'svelte'
  import { isCoreError, openDesktopProfile } from './core'
  import {
    desktopNotices,
    dismissDesktopNotice,
    onDesktopNotices,
    type DesktopNotice,
  } from './desktopNotices'
  import { errorMessage, t } from './i18n.svelte'

  let notices = $state<DesktopNotice[]>([])
  let busy = $state(false)
  let error = $state<string | null>(null)

  onMount(() => {
    desktopNotices()
      .then((n) => (notices = n.notices))
      .catch(() => {})
    const unlisten = onDesktopNotices((n) => (notices = n.notices))
    return () => {
      unlisten.then((stop) => stop())
    }
  })

  async function run(action: () => Promise<unknown>) {
    busy = true
    error = null
    try {
      await action()
    } catch (e) {
      error = errorMessage(isCoreError(e) ? e.code : 'unknown')
    } finally {
      busy = false
    }
  }
</script>

{#each notices as notice (notice.account)}
  <div class="banner" role="alert" style:--profile={notice.color}>
    <p>{notice.message}</p>
    <div class="actions">
      <button
        disabled={busy || !notice.can_reopen}
        onclick={() => run(() => openDesktopProfile(notice.account))}>{notice.reopen}</button
      >
      <button disabled={busy} onclick={() => run(() => dismissDesktopNotice(notice.account))}
        >{t('action.dismiss')}</button
      >
    </div>
  </div>
{/each}
{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.75rem;
    margin-bottom: 0.75rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--border);
    border-left: 4px solid var(--profile, var(--warn));
    border-radius: 0.4rem;
  }
  p {
    flex: 1 1 18rem;
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 0.4rem;
  }
  .error {
    margin: 0 0 0.75rem;
    color: var(--error);
  }
</style>
