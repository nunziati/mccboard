<script lang="ts">
  // The first sign-in guide of a desktop profile: sign in with a code, since the app's
  // claude:// link would land in the default profile. Shown while the profile has no account
  // signed in; it goes away by itself once one is observed.
  import { t } from './i18n.svelte'

  let { name, ondismiss }: { name: string; ondismiss: () => void } = $props()
</script>

<div class="guide" role="status">
  <h3>{t('desktop.guide.title', { name })}</h3>
  <ol>
    <li>{t('desktop.guide.step_email', { name })}</li>
    <li>{t('desktop.guide.step_link')}</li>
    <li><strong>{t('desktop.guide.step_cancel')}</strong></li>
    <li>{t('desktop.guide.step_code', { name })}</li>
  </ol>
  <p class="small">{t('desktop.guide.why')}</p>
  <p class="muted small">{t('desktop.guide.closes')}</p>
  <button onclick={ondismiss}>{t('action.dismiss')}</button>
</div>

<style>
  .guide {
    margin: 0.5rem 0 0.75rem;
    padding: 0.6rem 0.85rem;
    border: 1px solid var(--warn);
    border-radius: 0.4rem;
  }
  h3 {
    margin: 0 0 0.35rem;
    font-size: 1rem;
  }
  ol {
    margin: 0 0 0.5rem;
    padding-left: 1.4rem;
  }
  li {
    margin-bottom: 0.2rem;
  }
  p {
    margin: 0 0 0.4rem;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85em;
  }
</style>
