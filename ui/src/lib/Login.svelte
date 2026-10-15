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
        return t('login.token_expired')
      case 'not_logged_in':
        return t('login.not_logged_in')
      case 'unknown':
        return t('login.unknown')
    }
  }

  // An expired access token isn't a logout: Claude Code renews it whenever it runs.
  function details(login: LoginState): string[] {
    switch (login.state) {
      case 'logged_in':
        return [
          t('login.logged_in_until', { time: formatTime(login.expires_at) }),
          t('login.renews_hint'),
        ]
      case 'expired':
        return [t('login.token_expired_hint')]
      default:
        return []
    }
  }
</script>

<span class="login {observation.login.state}">{label(observation.login)}</span>
{#each details(observation.login) as detail (detail)}
  <div class="muted small">{detail}</div>
{/each}
{#each observation.warnings as warning (warning)}
  <div class="warning small">{t(`warning.${warning}`)}</div>
{/each}

<style>
  .login.logged_in {
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
