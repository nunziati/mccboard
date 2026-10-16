<script lang="ts">
  // "Remove" for a desktop profile: a dialog with the two levels. Removing from mccboard is the
  // default; deleting the data folder too needs an explicit confirmation that the sign-in on
  // this machine goes away. The default profile never gets this.
  import { t } from './i18n.svelte'

  let {
    name,
    folder,
    busy,
    onremove,
  }: {
    /** The profile's account, for the confirmation. */
    name: string
    folder: string
    busy: boolean
    onremove: (deleteFolder: boolean) => void
  } = $props()

  let dialog = $state<HTMLDialogElement | null>(null)
  let deleteFolder = $state(false)
  let confirmed = $state(false)

  function open() {
    deleteFolder = false
    confirmed = false
    dialog?.showModal()
  }

  function close() {
    dialog?.close()
  }

  function submit(event: SubmitEvent) {
    event.preventDefault()
    if (deleteFolder && !confirmed) return
    close()
    onremove(deleteFolder)
  }
</script>

<button disabled={busy} title={t('desktop.remove_hint')} onclick={open}>
  {t('action.remove')}
</button>

<dialog bind:this={dialog}>
  <form onsubmit={submit}>
    <h3>{t('desktop.remove.title', { name })}</h3>
    <p><code>{folder}</code></p>

    <label class="choice">
      <input type="radio" name="level" value={false} bind:group={deleteFolder} />
      <span>
        <strong>{t('remove.from_mccboard')}</strong>
        <span class="muted small">{t('desktop.remove.from_mccboard_hint')}</span>
      </span>
    </label>
    <label class="choice">
      <input type="radio" name="level" value={true} bind:group={deleteFolder} />
      <span>
        <strong>{t('desktop.remove.with_folder')}</strong>
        <span class="muted small">{t('desktop.remove.with_folder_hint')}</span>
      </span>
    </label>

    {#if deleteFolder}
      <label class="confirm">
        <input type="checkbox" bind:checked={confirmed} />
        <span>{t('desktop.remove.confirm_sign_in', { name })}</span>
      </label>
    {/if}

    <div class="buttons">
      <button type="button" onclick={close}>{t('action.cancel')}</button>
      <button type="submit" class:danger={deleteFolder} disabled={busy || (deleteFolder && !confirmed)}>
        {deleteFolder ? t('remove.confirm_delete') : t('remove.confirm')}
      </button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    max-width: 32rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    background: var(--bg);
    color: var(--fg);
    text-align: left;
    white-space: normal;
  }
  h3 {
    margin: 0 0 0.3rem;
    font-size: 1.05rem;
  }
  p {
    margin: 0 0 0.75rem;
  }
  .choice,
  .confirm {
    display: flex;
    gap: 0.5rem;
    align-items: flex-start;
    margin-bottom: 0.6rem;
  }
  .choice > span {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .confirm {
    color: var(--error);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }
  .danger {
    color: var(--error);
    font-weight: 600;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
