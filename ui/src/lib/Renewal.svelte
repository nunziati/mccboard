<script lang="ts">
  // Claude Code renewing an instance's expired token, as the core reports it (#30): when it
  // did, or why it couldn't and when it tries again.
  import { formatClock, t } from './i18n.svelte'
  import type { RenewalFailure, RenewalSituation } from './usage'

  let { renewal }: { renewal: RenewalSituation } = $props()

  function reason(failure: RenewalFailure): string {
    switch (failure.kind) {
      case 'exited':
        return failure.code === null
          ? t('renewal.failure.stopped')
          : t('renewal.failure.exited', { code: failure.code })
      default:
        return t(`renewal.failure.${failure.kind}`)
    }
  }

  function next(at: string | null): string {
    return at ? t('renewal.next_at', { time: formatClock(at) }) : t('renewal.next_refresh')
  }

  function text(renewal: RenewalSituation): { line: string; failed: boolean } | null {
    switch (renewal.state) {
      case 'renewed':
        return { line: t('renewal.renewed', { time: formatClock(renewal.at) }), failed: false }
      case 'failed':
        return {
          line: `${t('renewal.failed', { reason: reason(renewal.failure) })} ${next(renewal.retry_at)}`,
          failed: true,
        }
      case 'waiting':
        return {
          line: renewal.next_at ? next(renewal.next_at) : t('renewal.pending'),
          failed: false,
        }
      case 'off':
      case 'quiet':
        return null
    }
  }

  let shown = $derived(text(renewal))
</script>

{#if shown}
  <div class="small" class:warning={shown.failed} class:muted={!shown.failed}>{shown.line}</div>
{/if}

<style>
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
