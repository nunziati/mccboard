<script lang="ts">
  // "Add" a linked instance: the account label and, optionally, a folder other than the
  // default one the core derives from the label.
  import { addFolder } from './core'
  import { t } from './i18n.svelte'

  let {
    busy,
    onadd,
  }: {
    busy: boolean
    /** Adds the instance; resolves to whether it went through (the form then clears). */
    onadd: (label: string, path: string | null) => Promise<boolean>
  } = $props()

  let label = $state('')
  let folder = $state('')
  /** The default folder for the label typed, from the core; null when it gives none. */
  let defaultFolder = $state<string | null>(null)

  $effect(() => {
    const typed = label.trim()
    if (!typed) {
      defaultFolder = null
      return
    }
    let current = true
    addFolder(typed)
      .then((path) => {
        if (current) defaultFolder = path
      })
      .catch(() => {
        if (current) defaultFolder = null
      })
    return () => {
      current = false
    }
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    const typed = label.trim()
    if (!typed) return
    if (await onadd(typed, folder.trim() || null)) {
      label = ''
      folder = ''
    }
  }
</script>

<form class="add" onsubmit={submit}>
  <h3>{t('add.title')}</h3>
  <p class="muted small">{t('add.hint')}</p>
  <div class="fields">
    <label>
      {t('add.label')}
      <input bind:value={label} placeholder={t('add.label_placeholder')} size="10" />
    </label>
    <label class="folder">
      {t('add.folder')}
      <input bind:value={folder} placeholder={defaultFolder ?? ''} />
    </label>
    <button type="submit" disabled={busy || !label.trim()}>{t('action.add')}</button>
  </div>
  {#if defaultFolder && !folder.trim()}
    <p class="muted small">{t('add.folder_default', { path: defaultFolder })}</p>
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
