<script lang="ts">
  // "Add" a desktop profile: one of the accounts the core says can get one, and its data
  // folder, pre-filled with the core's default for that account and editable.
  import { desktopProfileFolder, type Account } from './core'
  import { t } from './i18n.svelte'

  let {
    candidates,
    busy,
    onadd,
  }: {
    /** The accounts with a local instance and no profile yet. */
    candidates: Account[]
    busy: boolean
    /** Adds the profile; resolves to whether it went through (the form then clears). */
    onadd: (uuid: string, folder: string | null) => Promise<boolean>
  } = $props()

  let account = $state('')
  let folder = $state('')

  // An account no longer offered (added elsewhere, instance removed) is unselected.
  $effect(() => {
    if (account && !candidates.some((c) => c.uuid === account)) account = ''
  })

  // Choosing an account fills in its default folder, which the user can then change.
  $effect(() => {
    const chosen = account
    folder = ''
    if (!chosen) return
    let current = true
    desktopProfileFolder(chosen)
      .then((path) => {
        if (current) folder = path
      })
      .catch(() => {})
    return () => {
      current = false
    }
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (!account) return
    if (await onadd(account, folder.trim() || null)) account = ''
  }
</script>

<form class="add" onsubmit={submit}>
  <h3>{t('desktop.add.title')}</h3>
  {#if candidates.length === 0}
    <p class="muted small">{t('desktop.add.none')}</p>
  {:else}
    <p class="muted small">{t('desktop.add.hint')}</p>
    <div class="fields">
      <label>
        {t('desktop.add.account')}
        <select bind:value={account}>
          <option value="">{t('desktop.add.choose')}</option>
          {#each candidates as candidate (candidate.uuid)}
            <option value={candidate.uuid}>
              {candidate.email ? `${candidate.label} (${candidate.email})` : candidate.label}
            </option>
          {/each}
        </select>
      </label>
      <label class="folder">
        {t('desktop.add.folder')}
        <input bind:value={folder} disabled={!account} />
      </label>
      <button type="submit" disabled={busy || !account}>{t('action.add')}</button>
    </div>
  {/if}
</form>

<style>
  .add {
    margin-top: 0.9rem;
  }
  .fields {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 0.35rem;
  }
  label {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  .folder {
    flex: 1;
  }
  .folder input {
    flex: 1;
    min-width: 14rem;
  }
  h3 {
    margin: 0.75rem 0 0.35rem;
    font-size: 1rem;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
