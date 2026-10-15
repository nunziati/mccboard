<script lang="ts">
  // An instance's drift count, with a toggle for the explained list.
  import type { InstanceDrift } from './core'
  import { t } from './i18n.svelte'

  let {
    drift,
    open,
    ontoggle,
  }: { drift: InstanceDrift | undefined; open: boolean; ontoggle: () => void } = $props()

  let count = $derived(drift?.drift_count ?? 0)
  let infos = $derived(drift ? drift.items.length - drift.drift_count : 0)
</script>

{#if drift}
  {#if count > 0}
    <span class="drift">{count === 1 ? t('drift.count_one') : t('drift.count', { count })}</span>
  {:else}
    <span class="clean">{t('drift.none')}</span>
  {/if}
  {#if infos > 0}
    <div class="muted small">{t('drift.notes', { count: infos })}</div>
  {/if}
  {#if drift.items.length > 0}
    <div>
      <button class="link" onclick={ontoggle} aria-expanded={open}>
        {open ? t('drift.hide') : t('drift.show')}
      </button>
    </div>
  {/if}
{/if}

<style>
  .drift {
    color: var(--warn);
    font-weight: 600;
  }
  .clean {
    color: var(--ok);
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    text-decoration: underline;
    cursor: pointer;
    font-size: 0.85em;
  }
</style>
