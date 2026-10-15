<script lang="ts">
  // An instance's launcher in the instances table: plain `claude` for the primary instance,
  // the launcher's name and state for a linked one.
  import type { InstanceKind, InstanceLauncher } from './core'
  import { t } from './i18n.svelte'

  let { kind, launcher }: { kind: InstanceKind; launcher: InstanceLauncher | undefined } = $props()
</script>

{#if kind === 'primary'}
  <code title={t('launcher.primary_hint')}>claude</code>
{:else if launcher}
  <code>{launcher.name}</code>
  {#if launcher.state === 'ok'}
    <div class="ok small">{t('launcher.state.ok')}</div>
  {:else if launcher.state === 'missing'}
    <div class="warning small">{t('launcher.state.missing')}</div>
  {:else}
    <div class="warning small">{t('launcher.state.stale')}</div>
    {#if launcher.found}
      <div class="muted small">{t('launcher.state.stale_found', { path: launcher.found })}</div>
    {/if}
  {/if}
{/if}

<style>
  .ok {
    color: var(--ok);
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
