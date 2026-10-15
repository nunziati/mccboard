<script lang="ts">
  import { onMount } from 'svelte'
  import { isCoreError, scan, type Candidate, type LoginState } from './lib/core'
  import { errorMessage, formatTime, i18n, LANGUAGE_NAMES, LANGUAGES, t, type Lang } from './lib/i18n.svelte'

  let candidates = $state<Candidate[] | null>(null)
  let errorCode = $state<string | null>(null)
  let scanning = $state(false)

  async function rescan() {
    scanning = true
    errorCode = null
    try {
      candidates = await scan()
    } catch (e) {
      errorCode = isCoreError(e) ? e.code : 'unknown'
    } finally {
      scanning = false
    }
  }

  function loginLabel(login: LoginState): string {
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

  function loginDetail(login: LoginState): string | null {
    switch (login.state) {
      case 'logged_in':
        return t('login.logged_in_until', { time: formatTime(login.expires_at) })
      case 'expired':
        return t('login.expired_hint')
      default:
        return null
    }
  }

  onMount(rescan)
</script>

<main>
  <header>
    <div>
      <h1>{t('instances.title')}</h1>
      <p class="muted">{t('instances.subtitle')}</p>
    </div>
    <div class="actions">
      <label>
        {t('language.label')}
        <select
          value={i18n.lang}
          onchange={(e) => (i18n.lang = (e.currentTarget as HTMLSelectElement).value as Lang)}
        >
          {#each LANGUAGES as lang (lang)}
            <option value={lang}>{LANGUAGE_NAMES[lang]}</option>
          {/each}
        </select>
      </label>
      <button onclick={rescan} disabled={scanning}>
        {scanning ? t('instances.scanning') : t('instances.rescan')}
      </button>
    </div>
  </header>

  {#if errorCode}
    <p class="error" role="alert">{errorMessage(errorCode)}</p>
  {:else if candidates && candidates.length === 0}
    <p class="muted">{t('instances.empty')}</p>
  {:else if candidates}
    <table>
      <thead>
        <tr>
          <th>{t('instances.column.path')}</th>
          <th>{t('instances.column.kind')}</th>
          <th>{t('instances.column.account')}</th>
          <th>{t('instances.column.login')}</th>
        </tr>
      </thead>
      <tbody>
        {#each candidates as candidate (candidate.config_dir)}
          <tr>
            <td><code>{candidate.config_dir}</code></td>
            <td>{candidate.kind === 'primary' ? t('kind.primary') : t('kind.linked')}</td>
            <td>
              {#if candidate.account}
                <div>{candidate.account.email ?? candidate.account.uuid}</div>
                {#if candidate.account.email}
                  <div class="muted small"><code>{candidate.account.uuid}</code></div>
                {/if}
              {:else}
                <span class="muted">{t('account.none')}</span>
              {/if}
            </td>
            <td>
              <span class="login {candidate.login.state}">{loginLabel(candidate.login)}</span>
              {#if loginDetail(candidate.login)}
                <div class="muted small">{loginDetail(candidate.login)}</div>
              {/if}
              {#each candidate.warnings as warning (warning)}
                <div class="warning small">{t(`warning.${warning}`)}</div>
              {/each}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>

<style>
  main {
    padding: 1.25rem 1.5rem;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1rem;
  }
  h1 {
    margin: 0 0 0.25rem;
    font-size: 1.4rem;
  }
  p {
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    flex-shrink: 0;
  }
  label {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  table {
    width: 100%;
    border-collapse: collapse;
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
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
  .login.logged_in {
    color: var(--ok);
  }
  .login.expired,
  .warning {
    color: var(--warn);
  }
  .error {
    color: var(--error);
  }
</style>
