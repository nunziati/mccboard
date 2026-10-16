<script lang="ts">
  // An account's label, color and launcher, editable in place.
  import { updateAccount, type Account } from './core'
  import { t } from './i18n.svelte'

  let {
    account,
    colorLocked = false,
    onerror,
  }: {
    account: Account
    /** The default desktop profile's account: always Claude's orange, not editable. */
    colorLocked?: boolean
    onerror: (error: unknown) => void
  } = $props()

  let editing = $state(false)
  let saving = $state(false)
  let label = $state('')
  let color = $state('')
  let launcher = $state('')

  function edit() {
    label = account.label
    color = account.color
    launcher = account.launcher ?? ''
    editing = true
  }

  async function save(event: SubmitEvent) {
    event.preventDefault()
    saving = true
    try {
      const saved = await updateAccount(account.uuid, {
        label: label === account.label ? undefined : label,
        color: colorLocked || color === account.color ? undefined : color,
        launcher: launcher === (account.launcher ?? '') ? undefined : launcher.trim(),
      })
      editing = false
      // Saved all the same; its message says so.
      if (saved.shortcuts_not_synced) onerror(saved.shortcuts_not_synced)
    } catch (e) {
      onerror(e)
    } finally {
      saving = false
    }
  }
</script>

{#if editing}
  <form class="account" onsubmit={save}>
    <label>
      {t('account.field.label')}
      <input bind:value={label} maxlength="40" required />
    </label>
    <label>
      {t('account.field.color')}
      <input
        type="color"
        bind:value={color}
        disabled={colorLocked}
        title={colorLocked ? t('account.color_locked') : undefined}
      />
    </label>
    <label>
      {t('account.field.launcher')}
      <input
        bind:value={launcher}
        placeholder={t('account.launcher_default', { name: account.launcher_name })}
      />
    </label>
    <button type="submit" disabled={saving}>{t('action.save')}</button>
    <button type="button" onclick={() => (editing = false)}>{t('action.cancel')}</button>
  </form>
{:else}
  <div class="account">
    <span class="swatch" style:background={account.color}></span>
    <strong>{account.label}</strong>
    {#if account.email}<span>{account.email}</span>{/if}
    <code class="muted small">{account.uuid}</code>
    <span class="muted small">{t('account.launcher', { name: account.launcher_name })}</span>
    <button onclick={edit}>{t('action.edit')}</button>
  </div>
{/if}

<style>
  .account {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    align-items: center;
  }
  label {
    display: flex;
    gap: 0.35rem;
    align-items: center;
  }
  .swatch {
    width: 0.9rem;
    height: 0.9rem;
    border-radius: 0.2rem;
    display: inline-block;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
