<script lang="ts">
  // An instance's drift, item by item: the problem in plain words and what a repair would do.
  // Kinds the UI has no translation for yet show the core's English wording.
  import type { DriftItem, Problem, Repair } from './core'
  import en from './locales/en.json'
  import { t, type Key } from './i18n.svelte'

  let {
    items,
    busy = false,
    onrepair,
  }: {
    items: DriftItem[]
    busy?: boolean
    /** Repairs one item by id; without it, no repair buttons are shown. */
    onrepair?: (id: string) => void
  } = $props()

  function repairable(item: DriftItem): boolean {
    return !item.informational && item.repair.action !== 'nothing'
  }

  function translated(key: string, vars: Record<string, string>, fallback: string): string {
    return key in en ? t(key as Key, vars) : fallback
  }

  function problemKey(item: DriftItem, problem: Problem): string {
    switch (problem.kind) {
      case 'broken_link':
        return problem.target === null ? 'drift.problem.broken_link_unreadable' : 'drift.problem.broken_link'
      case 'whole_folder_link':
        return `drift.problem.whole_folder_link.${item.item}`
      default:
        return `drift.problem.${problem.kind}`
    }
  }

  function problemText(item: DriftItem): string {
    const problem = item.problem as Problem & { target?: string | null; skill?: string }
    const vars: Record<string, string> = { item: item.item }
    if (problem.target) vars.target = problem.target
    if (problem.skill) vars.skill = problem.skill
    return translated(problemKey(item, problem), vars, item.explanation)
  }

  function repairText(item: DriftItem): string {
    const repair = item.repair as Repair & { target?: string; skills?: string[] }
    const vars: Record<string, string> = { item: item.item }
    if (repair.target) vars.target = repair.target
    let key = `drift.repair.${repair.action}`
    if (repair.skills) {
      vars.skills = repair.skills.join(', ')
      if (repair.skills.length === 0) key = 'drift.repair.migrate_skills_none'
    }
    return translated(key, vars, item.repair_explanation)
  }
</script>

<ul class="drift">
  {#each items as item (item.id)}
    <li class:info={item.informational}>
      <div>
        {#if item.informational}<span class="tag">{t('drift.informational')}</span>{/if}
        {problemText(item)}
      </div>
      <div class="muted small">{t('drift.repair', { repair: repairText(item) })}</div>
      <div class="muted small">
        <code>{item.id}</code>
        {#if onrepair && repairable(item)}
          <button class="small" disabled={busy} onclick={() => onrepair(item.id)}>
            {t('action.repair_item')}
          </button>
        {/if}
      </div>
    </li>
  {/each}
</ul>

<style>
  .drift {
    margin: 0;
    padding-left: 1.2rem;
  }
  li {
    margin: 0.35rem 0;
    color: var(--warn);
  }
  li.info {
    color: inherit;
  }
  li > div:first-child {
    color: inherit;
  }
  .tag {
    font-size: 0.8em;
    padding: 0 0.35rem;
    margin-right: 0.3rem;
    border: 1px solid var(--border);
    border-radius: 0.3rem;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
