<script lang="ts">
  import { onMount } from 'svelte'
  import AccountHeader from './lib/AccountHeader.svelte'
  import DriftCount from './lib/DriftCount.svelte'
  import DriftList from './lib/DriftList.svelte'
  import LauncherCell from './lib/LauncherCell.svelte'
  import Launchers from './lib/Launchers.svelte'
  import Login from './lib/Login.svelte'
  import RemoveInstance from './lib/RemoveInstance.svelte'
  import RepairResult from './lib/RepairResult.svelte'
  import SharingQuestions from './lib/SharingQuestions.svelte'
  import {
    answerSharing,
    doctor as loadDoctor,
    type Doctor,
    type SharingAnswer,
    isCoreError,
    launchers as loadLaunchers,
    onConfigChanged,
    register,
    registry as loadRegistry,
    removeInstance,
    repair,
    type RepairReport,
    scan,
    syncLaunchers,
    type Account,
    type Candidate,
    type CoreError,
    type InstanceStatus,
    type LauncherStatus,
    type Registry,
  } from './lib/core'
  import {
    chooseLanguage,
    errorMessage,
    i18n,
    LANGUAGE_NAMES,
    LANGUAGES,
    loadLanguage,
    t,
    type Lang,
  } from './lib/i18n.svelte'

  /** Instances shown under one heading: an account, an account missing from config.toml, or none. */
  interface Group {
    key: string
    account: Account | null
    uuid: string | null
    instances: InstanceStatus[]
  }

  let registry = $state<Registry | null>(null)
  let candidates = $state<Candidate[] | null>(null)
  let launcherStatus = $state<LauncherStatus | null>(null)
  /** An error reading the launchers: shown in their section, the rest of the window still works. */
  let launcherError = $state<CoreError | null>(null)
  /** An error loading the window's data: shown instead of the lists. */
  let loadError = $state<CoreError | null>(null)
  /** An error from the last action: shown above the lists until dismissed. */
  let actionError = $state<CoreError | null>(null)
  let notice = $state<string | null>(null)
  let busy = $state(false)
  let pathToRegister = $state('')
  let doctor = $state<Doctor | null>(null)
  /** Instances whose explained drift list is open, by config directory. */
  let openDrift = $state<Record<string, boolean>>({})
  /** What the last repair did, until dismissed. */
  let repairReport = $state<RepairReport | null>(null)

  function repairInstance(path: string, item?: string) {
    repairReport = null
    return act(async () => {
      repairReport = await repair(path, item)
      return null
    })
  }

  function driftOf(instance: InstanceStatus) {
    return doctor?.instances.find((d) => d.config_dir === instance.config_dir)
  }

  function answer(item: string, sharing: SharingAnswer) {
    return act(async () => {
      await answerSharing(item, sharing)
      return t(sharing === 'link' ? 'notice.shared_link' : 'notice.shared_private', { item })
    })
  }

  let groups = $derived(registry ? groupByAccount(registry) : [])
  let proposals = $derived(candidates?.filter((c) => !c.registered) ?? [])

  function asCoreError(e: unknown): CoreError {
    return isCoreError(e) ? e : { code: 'unknown' }
  }

  function groupByAccount(registry: Registry): Group[] {
    const groups: Group[] = registry.accounts.map((account) => ({
      key: account.uuid,
      account,
      uuid: account.uuid,
      instances: registry.instances.filter((i) => i.registered_account === account.uuid),
    }))
    for (const instance of registry.instances) {
      const uuid = instance.registered_account
      if (registry.accounts.some((a) => a.uuid === uuid)) continue
      const group = groups.find((g) => g.account === null && g.uuid === uuid)
      if (group) group.instances.push(instance)
      else groups.push({ key: uuid ?? 'none', account: null, uuid, instances: [instance] })
    }
    // Accounts first, then accounts missing from config.toml, then instances with none.
    return groups.sort((a, b) => rank(a) - rank(b))
  }

  function rank(group: Group): number {
    return group.account ? 0 : group.uuid ? 1 : 2
  }

  function accountName(uuid: string): string {
    return registry?.accounts.find((a) => a.uuid === uuid)?.label ?? uuid
  }

  async function reload() {
    try {
      const [nextRegistry, nextCandidates, nextDoctor] = await Promise.all([
        loadRegistry(),
        scan(),
        loadDoctor(),
      ])
      registry = nextRegistry
      candidates = nextCandidates
      doctor = nextDoctor
      loadError = null
    } catch (e) {
      loadError = asCoreError(e)
    }
    try {
      launcherStatus = await loadLaunchers()
      launcherError = null
    } catch (e) {
      launcherStatus = null
      launcherError = asCoreError(e)
    }
  }

  async function act(action: () => Promise<string | null>) {
    busy = true
    actionError = null
    notice = null
    try {
      notice = await action()
    } catch (e) {
      actionError = asCoreError(e)
    } finally {
      busy = false
      await reload()
    }
  }

  function registerPath(path: string) {
    return act(async () => {
      const registration = await register(path)
      const done = t('notice.registered', { path: registration.instance.config_dir })
      return registration.new_account
        ? `${done} ${t('notice.new_account', { label: registration.new_account.label })}`
        : done
    })
  }

  function remove(path: string, deleteFolder: boolean) {
    return act(async () => {
      const removal = await removeInstance(path, deleteFolder)
      const key =
        removal.folder === 'deleted'
          ? 'notice.removed_deleted'
          : removal.folder === 'already_gone'
            ? 'notice.removed_already_gone'
            : 'notice.removed'
      return t(key, { path: removal.config_dir })
    })
  }

  /** Who is logged in to an instance, for confirming its deletion. */
  function loginOf(instance: InstanceStatus): string | null {
    const account = instance.account
    if (!account) return null
    const label = registry?.accounts.find((a) => a.uuid === account.uuid)?.label
    const name = account.email ?? account.uuid
    return label ? `${label} (${name})` : name
  }

  function fixLaunchers() {
    return act(async () => {
      const sync = await syncLaunchers()
      const changed = sync.created.length + sync.updated.length + sync.removed.length
      const notices = [
        changed === 0
          ? t('notice.launchers_up_to_date')
          : t('notice.launchers_synced', {
              created: sync.created.length,
              updated: sync.updated.length,
              removed: sync.removed.length,
            }),
      ]
      const place = launcherStatus?.place.path ?? ''
      if (sync.user_path?.status === 'added') notices.push(t('notice.path_added', { path: place }))
      else if (sync.user_path?.status === 'not_added') notices.push(t('notice.path_not_added', { path: place }))
      else if (changed > 0) notices.push(t('notice.new_terminal'))
      return notices.join(' ')
    })
  }

  function launcherOf(instance: InstanceStatus) {
    return launcherStatus?.instances.find((l) => l.config_dir === instance.config_dir)
  }

  async function registerTyped(event: SubmitEvent) {
    event.preventDefault()
    const path = pathToRegister.trim()
    if (!path) return
    await registerPath(path)
    if (!actionError) pathToRegister = ''
  }

  function observedName(instance: InstanceStatus): string {
    const account = instance.account
    return account ? (account.email ?? account.uuid) : ''
  }

  onMount(() => {
    loadLanguage()
    reload()
    const unlisten = onConfigChanged(() => {
      loadLanguage()
      reload()
    })
    return () => {
      unlisten.then((stop) => stop())
    }
  })
</script>

{#snippet failure(error: CoreError, dismiss: (() => void) | null)}
  <div class="error" role="alert">
    <p>{errorMessage(error.code)}</p>
    {#if error.detail}<p class="small"><code>{error.detail}</code></p>{/if}
    {#if error.path}<p class="small"><code>{error.path}</code></p>{/if}
    {#if error.processes?.length}
      <ul class="small">
        {#each error.processes as process (process.pid)}
          <li>
            {process.cwd
              ? t('error.process_in', { name: process.name, pid: process.pid, cwd: process.cwd })
              : t('error.process', { name: process.name, pid: process.pid })}
          </li>
        {/each}
      </ul>
    {/if}
    {#if dismiss}<button onclick={dismiss}>{t('action.dismiss')}</button>{/if}
  </div>
{/snippet}

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
          onchange={(e) =>
            chooseLanguage((e.currentTarget as HTMLSelectElement).value as Lang).catch(
              (error: unknown) => (actionError = asCoreError(error)),
            )}
        >
          {#each LANGUAGES as lang (lang)}
            <option value={lang}>{LANGUAGE_NAMES[lang]}</option>
          {/each}
        </select>
      </label>
      <button onclick={reload} disabled={busy}>{t('instances.rescan')}</button>
    </div>
  </header>

  {#if loadError}
    {@render failure(loadError, null)}
  {:else}
    {#if actionError}
      {@render failure(actionError, () => (actionError = null))}
    {/if}
    {#if notice}
      <p class="notice" role="status">{notice}</p>
    {/if}
    {#if repairReport}
      <RepairResult report={repairReport} ondismiss={() => (repairReport = null)} />
    {/if}

    <SharingQuestions questions={doctor?.questions ?? []} {busy} onanswer={answer} />

    <section>
      <h2>{t('section.registered')}</h2>
      {#if registry && groups.length === 0}
        <p class="muted">{t('registered.empty')}</p>
      {/if}
      {#each groups as group (group.key)}
        <div class="group">
          {#if group.account}
            <AccountHeader account={group.account} onerror={(e) => (actionError = asCoreError(e))} />
          {:else if group.uuid}
            <div><strong>{t('account.unknown_group', { uuid: group.uuid })}</strong></div>
            <p class="muted small">{t('account.unknown_hint')}</p>
          {:else}
            <div><strong>{t('account.no_account_group')}</strong></div>
            <p class="muted small">{t('account.no_account_hint')}</p>
          {/if}
          {#if group.instances.length === 0}
            <p class="muted small">{t('account.no_instances')}</p>
          {:else}
            <table>
              <thead>
                <tr>
                  <th>{t('instances.column.path')}</th>
                  <th>{t('instances.column.kind')}</th>
                  <th>{t('instances.column.login')}</th>
                  <th>{t('instances.column.drift')}</th>
                  <th>{t('instances.column.launcher')}</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {#each group.instances as instance (instance.config_dir)}
                  {@const drift = driftOf(instance)}
                  <tr>
                    <td><code>{instance.config_dir}</code></td>
                    <td>{instance.kind === 'primary' ? t('kind.primary') : t('kind.linked')}</td>
                    <td>
                      <Login observation={instance} />
                      {#if instance.account_status.status === 'changed'}
                        <div class="warning small">
                          {t('status.changed', {
                            account: observedName(instance),
                            registered: accountName(instance.account_status.registered),
                          })}
                        </div>
                      {:else if instance.account_status.status === 'unrecorded'}
                        <div class="warning small">
                          {t('status.unrecorded', { account: observedName(instance) })}
                        </div>
                      {/if}
                    </td>
                    <td>
                      <DriftCount
                        {drift}
                        open={!!openDrift[instance.config_dir]}
                        ontoggle={() =>
                          (openDrift[instance.config_dir] = !openDrift[instance.config_dir])}
                      />
                    </td>
                    <td><LauncherCell kind={instance.kind} launcher={launcherOf(instance)} /></td>
                    <td class="row-actions">
                      {#if instance.kind === 'linked' && drift && drift.drift_count > 0}
                        <button
                          disabled={busy}
                          title={t('action.repair_hint')}
                          onclick={() => repairInstance(instance.config_dir)}
                        >
                          {t('action.repair')}
                        </button>
                      {/if}
                      {#if instance.account_status.status === 'changed'}
                        <button disabled={busy} onclick={() => registerPath(instance.config_dir)}>
                          {t('action.accept_account')}
                        </button>
                      {:else if instance.account_status.status === 'unrecorded'}
                        <button disabled={busy} onclick={() => registerPath(instance.config_dir)}>
                          {t('action.record_account')}
                        </button>
                      {/if}
                      {#if instance.kind === 'linked'}
                        <RemoveInstance
                          path={instance.config_dir}
                          account={loginOf(instance)}
                          {busy}
                          onremove={(deleteFolder) => remove(instance.config_dir, deleteFolder)}
                        />
                      {/if}
                    </td>
                  </tr>
                  {#if drift && openDrift[instance.config_dir] && drift.items.length > 0}
                    <tr>
                      <td colspan="6">
                        <DriftList
                          items={drift.items}
                          {busy}
                          onrepair={instance.kind === 'linked'
                            ? (id) => repairInstance(instance.config_dir, id)
                            : undefined}
                        />
                      </td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      {/each}

      <h3>{t('launchers.title')}</h3>
      {#if launcherError}
        {@render failure(launcherError, null)}
      {:else if launcherStatus}
        <Launchers status={launcherStatus} {busy} onsync={fixLaunchers} />
      {/if}

      <form class="register-path" onsubmit={registerTyped}>
        <label for="register-path">{t('register_path.label')}</label>
        <input
          id="register-path"
          bind:value={pathToRegister}
          placeholder={t('register_path.placeholder')}
        />
        <button type="submit" disabled={busy || !pathToRegister.trim()}>
          {t('action.register')}
        </button>
      </form>
    </section>

    <section>
      <h2>{t('section.proposals')}</h2>
      <p class="muted small">{t('section.proposals_hint')}</p>
      {#if candidates && candidates.length === 0}
        <p class="muted">{t('instances.empty')}</p>
      {:else if candidates && proposals.length === 0}
        <p class="muted">{t('proposals.none')}</p>
      {:else if proposals.length > 0}
        <table>
          <thead>
            <tr>
              <th>{t('instances.column.path')}</th>
              <th>{t('instances.column.kind')}</th>
              <th>{t('instances.column.account')}</th>
              <th>{t('instances.column.login')}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each proposals as candidate (candidate.config_dir)}
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
                <td><Login observation={candidate} /></td>
                <td class="row-actions">
                  <button disabled={busy} onclick={() => registerPath(candidate.config_dir)}>
                    {t('action.register')}
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>
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
  h2 {
    margin: 1.25rem 0 0.5rem;
    font-size: 1.1rem;
  }
  h3 {
    margin: 0.75rem 0 0.35rem;
    font-size: 1rem;
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
  .group {
    margin-bottom: 1rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--border);
    border-radius: 0.4rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 0.4rem;
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
  .row-actions {
    text-align: right;
    white-space: nowrap;
  }
  .register-path {
    margin-top: 0.9rem;
    display: flex;
    gap: 0.5rem;
    align-items: center;
    flex-wrap: wrap;
  }
  .register-path input {
    flex: 1;
    min-width: 16rem;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
  .warning {
    color: var(--warn);
  }
  .notice {
    color: var(--ok);
    margin-bottom: 0.75rem;
  }
  .error {
    color: var(--error);
    margin-bottom: 0.75rem;
  }
</style>
