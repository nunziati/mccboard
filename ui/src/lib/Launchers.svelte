<script lang="ts">
  // Where the launchers live and what needs fixing, with the action that fixes it.
  import type { LauncherStatus } from './core'
  import { t } from './i18n.svelte'

  let { status, busy, onsync }: { status: LauncherStatus; busy: boolean; onsync: () => void } =
    $props()

  let offPath = $derived(
    status.place.kind === 'bin_folder' && !status.place.on_user_path && status.instances.length > 0,
  )
  let drift = $derived(
    status.instances.filter((i) => i.state !== 'ok').length + status.orphans.length + (offPath ? 1 : 0),
  )
</script>

<div class="launchers">
  <p class="muted small">
    {#if status.instances.length === 0 && status.orphans.length === 0}
      {t('launchers.none')}
    {:else if status.place.kind === 'script_folder'}
      {t('launchers.place.script_folder', { path: status.place.path })}
    {:else}
      {t('launchers.place.bin_folder', { path: status.place.path })}
    {/if}
  </p>
  {#if offPath}
    <p class="warning small">{t('launchers.not_on_path')}</p>
  {/if}
  {#each status.orphans as orphan (orphan.name)}
    <p class="warning small">{t('launchers.orphan', { name: orphan.name })}</p>
  {/each}
  {#if drift > 0}
    <p class="small">
      <span class="warning">{t('launchers.drift', { count: drift })}</span>
      <button disabled={busy} onclick={onsync}>{t('action.sync_launchers')}</button>
    </p>
  {/if}
</div>

<style>
  .launchers {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  p {
    margin: 0;
  }
  .warning {
    color: var(--warn);
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
