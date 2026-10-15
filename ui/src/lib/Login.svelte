<script lang="ts">
  // An instance's login state and warnings, as observed by the core.
  import type { LoginState, Observation } from './core'
  import { formatTime, t } from './i18n.svelte'

  let { observation }: { observation: Observation } = $props()

  function label(login: LoginState): string {
    switch (login.state) {
      case 'logged_in':
        return t('login.logged_in')
      case 'expired':
        return t('login.expired')
      case 'not_logged_in':
        return t('login.not_logged_in')
      case 'unknown':
        return t('login.unknown')
    }
  }

  function detail(login: LoginState): string | null {
    switch (login.state) {
      case 'logged_in':
        return t('login.logged_in_until', { time: formatTime(login.expires_at) })
      case 'expired':
        return t('login.expired_hint')
      default:
        return null
    }
  }
</script>

<span class="login {observation.login.state}">{label(observation.login)}</span>
{#if detail(observation.login)}
  <div class="muted small">{detail(observation.login)}</div>
{/if}
{#each observation.warnings as warning (warning)}
  <div class="warning small">{t(`warning.${warning}`)}</div>
{/each}

<style>
  .login.logged_in {
    color: var(--ok);
  }
  .login.expired,
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
