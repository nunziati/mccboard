<script lang="ts">
  // Unknown items of the primary instance, each asked about once: link it, or keep it private.
  import type { SharingAnswer, SharingQuestion } from './core'
  import { t } from './i18n.svelte'

  let {
    questions,
    busy,
    onanswer,
  }: {
    questions: SharingQuestion[]
    busy: boolean
    onanswer: (item: string, answer: SharingAnswer) => void
  } = $props()
</script>

{#if questions.length > 0}
  <section class="questions">
    <h2>{t('sharing.title')}</h2>
    <p class="muted small">{t('sharing.hint')}</p>
    <ul>
      {#each questions as question (question.item)}
        <li>
          <span>
            {t(question.is_dir ? 'sharing.question_folder' : 'sharing.question_file', {
              item: question.item,
            })}
          </span>
          <span class="buttons">
            <button disabled={busy} onclick={() => onanswer(question.item, 'link')}>
              {t('action.share_link')}
            </button>
            <button disabled={busy} onclick={() => onanswer(question.item, 'private')}>
              {t('action.share_private')}
            </button>
          </span>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .questions {
    margin-bottom: 1rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--warn);
    border-radius: 0.4rem;
  }
  h2 {
    margin: 0 0 0.25rem;
    font-size: 1.1rem;
  }
  p {
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    padding: 0.3rem 0;
  }
  .buttons {
    display: flex;
    gap: 0.4rem;
    flex-shrink: 0;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
