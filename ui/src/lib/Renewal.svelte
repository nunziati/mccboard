<script lang="ts">
  // Claude Code renewing an instance's expired token, as the core reports it (#30): when it
  // did, or why it couldn't and when it tries again.
  import { formatClock, t } from './i18n.svelte'
  import type { RenewalFailure, TokenRenewal } from './usage'

  let { renewal, expired }: { renewal: TokenRenewal; expired: boolean } = $props()

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

  function next(renewal: TokenRenewal): string {
    return renewal.next_attempt_at
      ? t('renewal.next_at', { time: formatClock(renewal.next_attempt_at) })
      : t('renewal.next_refresh')
  }

  function text(renewal: TokenRenewal, expired: boolean): { line: string; failed: boolean } | null {
    const last = renewal.last
    if (!expired) {
      return last?.outcome === 'renewed'
        ? { line: t('renewal.renewed', { time: formatClock(last.at) }), failed: false }
        : null
    }
    if (last?.outcome === 'failed') {
      return { line: `${t('renewal.failed', { reason: reason(last.failure) })} ${next(renewal)}`, failed: true }
    }
    if (renewal.next_attempt_at) return { line: next(renewal), failed: false }
    return { line: t('renewal.pending'), failed: false }
  }

  let shown = $derived(text(renewal, expired))
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
