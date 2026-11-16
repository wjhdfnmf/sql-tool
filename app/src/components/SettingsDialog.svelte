<script lang="ts">
  import { onMount } from 'svelte';
  import { applyTheme, defaultSettings, settings, themeGroups, themes } from '../lib/settings.svelte';
  import { asUiError, type Settings } from '../lib/types';

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let dialog: HTMLDialogElement;
  let draft = $state<Settings>({ ...defaultSettings });
  let saving = $state(false);
  let error = $state<string | null>(null);

  const rowLimits: { label: string; value: number | null }[] = [
    { label: '1,000 rows', value: 1000 },
    { label: '10,000 rows', value: 10_000 },
    { label: '100,000 rows', value: 100_000 },
    { label: 'No limit', value: null },
  ];

  onMount(() => {
    draft = { ...$state.snapshot(settings.current) };
    dialog.showModal();
  });

  // The theme is shown as soon as it is picked, so it can be judged before saving.
  $effect(() => {
    applyTheme(draft.theme);
  });

  function cancel() {
    applyTheme(settings.current.theme);
    onclose();
  }

  async function save() {
    saving = true;
    error = null;
    try {
      await settings.save($state.snapshot(draft));
      onclose();
    } catch (e) {
      error = asUiError(e).message;
      saving = false;
    }
  }

  function restoreDefaults() {
    draft = { ...defaultSettings };
  }
</script>

<dialog bind:this={dialog} oncancel={cancel} aria-labelledby="settings-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <h2 id="settings-title">Settings</h2>

    <div class="scroll">
      <section>
        <h3>Appearance</h3>
        <label>
          <span>Theme</span>
          <select bind:value={draft.theme}>
            {#each themeGroups as group (group)}
              <optgroup label={group}>
                {#each themes.filter((t) => t.group === group) as theme (theme.value)}
                  <option value={theme.value}>{theme.label}</option>
                {/each}
              </optgroup>
            {/each}
          </select>
          <span class="muted note">{themes.find((t) => t.value === draft.theme)?.note}</span>
        </label>
        <label>
          <span>Size of text and controls</span>
          <select bind:value={draft.zoom}>
            {#each [80, 90, 100, 110, 125, 150, 175] as percent}
              <option value={percent}>{percent}%</option>
            {/each}
          </select>
        </label>
      </section>

      <section>
        <h3>Editor</h3>
        <label>
          <span>Indent</span>
          <select bind:value={draft.editorTabSize}>
            <option value={2}>2 spaces</option>
            <option value={4}>4 spaces</option>
            <option value={8}>8 spaces</option>
          </select>
        </label>
        <label class="check"><input type="checkbox" bind:checked={draft.editorWordWrap} /> Wrap long lines</label>
        <label class="check"><input type="checkbox" bind:checked={draft.uppercaseKeywords} /> Offer keywords in UPPER CASE when completing</label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.rememberTabs} /> Bring back open query tabs when a connection is opened again
        </label>
        <p class="muted note">
          The text of open tabs is kept on this computer, not encrypted. A tab holding a statement with a password is never kept.
        </p>
      </section>

      <section>
        <h3>Queries</h3>
        <label>
          <span>Row limit for a new connection</span>
          <select bind:value={draft.defaultRowLimit}>
            {#each rowLimits as option}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
          <span class="muted note">At the limit the query is stopped on the server. It can be changed per connection in the toolbar.</span>
        </label>
        <label>
          <span>Stop a query after (seconds)</span>
          <input type="number" min="0" max="86400" step="1" bind:value={draft.queryTimeoutSecs} />
          <span class="muted note">
            0 means never: a query runs until it finishes or Stop is pressed. Otherwise a statement that runs longer is interrupted on
            the server, and the rest of the script is not run.
          </span>
        </label>
        <label>
          <span>Memory for results before using disk (MB)</span>
          <input type="number" min="64" max="16384" step="64" bind:value={draft.resultMemoryMb} />
          <span class="muted note">Shared by all open results. Beyond it, rows go to a temporary file that is removed when the result closes.</span>
        </label>
      </section>

      <section>
        <h3>Connections</h3>
        <label>
          <span>Give up connecting after (seconds)</span>
          <input type="number" min="3" max="300" step="1" bind:value={draft.connectTimeoutSecs} />
        </label>
        <label>
          <span>Keep idle connections alive every (seconds)</span>
          <input type="number" min="0" max="3600" step="30" bind:value={draft.keepAliveSecs} />
          <span class="muted note">
            0 means off. Servers close connections that sit idle; a closed one is reopened the next time it is used, but session
            variables and temporary tables are lost. A keep-alive of a few minutes prevents that, at the cost of holding the connections
            open for as long as the app is.
          </span>
        </label>
      </section>

      <section>
        <h3>Privacy</h3>
        <label class="check"><input type="checkbox" bind:checked={draft.keepHistory} /> Keep a history of executed statements</label>
        <p class="muted note">
          The history is kept on this computer, not encrypted. Statements that contain a password are never stored in full. Turning this
          off stops recording; the History tab's Clear removes what is already there.
        </p>
      </section>
    </div>

    {#if error}<p class="error-text" role="alert">{error}</p>{/if}

    <div class="buttons">
      <button type="button" onclick={restoreDefaults}>Restore Defaults</button>
      <span class="spacer"></span>
      <button type="button" onclick={cancel}>Cancel</button>
      <button type="submit" class="primary" disabled={saving}>{saving ? 'Saving…' : 'Save'}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(620px, calc(100vw - 32px));
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px;
    max-height: calc(100vh - 80px);
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding-right: 6px;
  }

  section {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  h3 {
    margin: 0;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  label > span:first-child {
    font-weight: 600;
  }

  label select,
  label input[type='number'] {
    max-width: 260px;
  }

  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  .note {
    font-size: 12px;
  }

  p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .buttons {
    display: flex;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }
</style>
