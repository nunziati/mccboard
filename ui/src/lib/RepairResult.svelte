<script lang="ts">
  // What the last repair did: items repaired (with where a replaced copy was backed up) and
  // items that failed, with the reason.
  import type { RepairReport } from './core'
  import { t } from './i18n.svelte'

  let { report, ondismiss }: { report: RepairReport; ondismiss: () => void } = $props()

  let failed = $derived(report.items.filter((i) => i.outcome.outcome === 'failed'))
  let done = $derived(report.items.length - failed.length)
</script>

<div class="result" class:failed={failed.length > 0} role="status">
  {#if report.items.length === 0}
    <p>{t('repair.nothing', { path: report.config_dir })}</p>
  {:else}
    <p>
      {done === 1
        ? t('repair.done_one', { path: report.config_dir })
        : t('repair.done', { count: done, path: report.config_dir })}
      {#if report.drift.drift_count > 0}
        {t('repair.left', { count: report.drift.drift_count })}
      {/if}
    </p>
    <ul>
      {#each report.items as item (item.id)}
        <li>
          <code>{item.id}</code>
          {#if item.outcome.outcome === 'failed'}
            <span class="bad">{t('repair.item_failed', { reason: item.outcome.reason })}</span>
          {:else if item.outcome.backup}
            <span class="muted">{t('repair.item_backup', { backup: item.outcome.backup })}</span>
          {:else}
            <span class="muted">{t('repair.item_done')}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  <button onclick={ondismiss}>{t('action.dismiss')}</button>
</div>

<style>
  .result {
    margin-bottom: 0.75rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--border);
    border-left: 3px solid var(--ok);
    border-radius: 0.4rem;
  }
  .result.failed {
    border-left-color: var(--error);
  }
  p {
    margin: 0 0 0.3rem;
  }
  ul {
    margin: 0 0 0.4rem;
    padding-left: 1.2rem;
  }
  .bad {
    color: var(--error);
  }
  .muted {
    color: var(--muted);
  }
</style>
