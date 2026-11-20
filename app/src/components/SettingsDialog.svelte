<script lang="ts" module>
  /** The tab that was in front the last time Settings was open, so it opens there again. */
  let lastTab = 'appearance';
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { REFORMAT_STYLES } from '../lib/reformat';
  import { applyTheme, defaultSettings, settings, themeGroups, themes } from '../lib/settings.svelte';
  import { app } from '../lib/state.svelte';
  import { asUiError, type Settings } from '../lib/types';

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let dialog: HTMLDialogElement;
  let draft = $state<Settings>({ ...defaultSettings });
  let saving = $state(false);
  let error = $state<string | null>(null);
  let tab = $state(lastTab);

  // A tab that is not there until it is found: the heading of Queries
  // clicked five times, or the word typed anywhere in this window.
  const WORD = 'threadripper';
  const THREADS = [1, 2, 4, 6, 8, 12, 16];
  let found = $state(false);
  let typed = '';
  let knocks = 0;

  function listen(e: KeyboardEvent) {
    if (e.key.length !== 1) return;
    typed = (typed + e.key.toLowerCase()).slice(-WORD.length);
    if (typed === WORD) reveal();
  }

  function knock() {
    if (++knocks >= 5) reveal();
  }

  function reveal() {
    found = true;
    tab = 'threads';
  }

  const tabs = $derived([
    { id: 'appearance', label: 'Appearance', mark: '◐' },
    { id: 'editor', label: 'Editor', mark: '✎' },
    { id: 'queries', label: 'Queries', mark: '▶' },
    { id: 'connections', label: 'Connections', mark: '⇄' },
    { id: 'privacy', label: 'Privacy', mark: '⚿' },
    ...(found || draft.sliceThreads > 1 ? [{ id: 'threads', label: 'Threadripper', mark: '⇉' }] : []),
  ]);

  /** How many settings on a tab differ from their defaults, to show where things have been changed. */
  const KEYS: Record<string, (keyof Settings)[]> = {
    appearance: ['theme', 'zoom'],
    editor: ['editorTabSize', 'editorWordWrap', 'uppercaseKeywords', 'reformatStyle', 'reformatKeywords'],
    queries: ['defaultRowLimit', 'queryTimeoutSecs', 'resultMemoryMb'],
    connections: ['organizeConnections', 'connectTimeoutSecs', 'keepAliveSecs'],
    privacy: ['keepHistory', 'rememberTabs'],
    threads: ['sliceThreads'],
  };
  const changed = (id: string) => (KEYS[id] ?? []).filter((key) => draft[key] !== defaultSettings[key]).length;

  const rowLimits: { label: string; value: number | null }[] = [
    { label: '1,000 rows', value: 1000 },
    { label: '10,000 rows', value: 10_000 },
    { label: '100,000 rows', value: 100_000 },
    { label: 'No limit', value: null },
  ];

  onMount(() => {
    draft = { ...$state.snapshot(settings.current) };
    if (tab === 'threads' && draft.sliceThreads <= 1) tab = 'appearance';
    dialog.showModal();
  });

  $effect(() => {
    lastTab = tab;
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

  /** Leaves Settings for the list of MySQL Workbench's connections. Changes made here and not saved are dropped. */
  function importConnections() {
    cancel();
    app.showImport = true;
  }

  /** Puts the settings of the tab in front back to their defaults; the other tabs are left alone. */
  function restoreTab() {
    for (const key of KEYS[tab] ?? []) (draft[key] as Settings[typeof key]) = defaultSettings[key];
  }

  /** Up and down move along the tabs, as in any list of tabs. */
  function tabKey(e: KeyboardEvent) {
    const at = tabs.findIndex((t) => t.id === tab);
    const to = e.key === 'ArrowDown' ? at + 1 : e.key === 'ArrowUp' ? at - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : -1;
    if (to < 0 || to >= tabs.length) return;
    e.preventDefault();
    tab = tabs[to].id;
    document.getElementById(`settings-tab-${tab}`)?.focus();
  }
</script>

<dialog bind:this={dialog} oncancel={cancel} onkeydown={listen} aria-labelledby="settings-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <div class="frame">
      <nav>
        <h2 id="settings-title">Settings</h2>
        <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
        <ul role="tablist" aria-orientation="vertical" aria-label="Settings">
          {#each tabs as t (t.id)}
            <li role="presentation">
              <button
                type="button"
                role="tab"
                id={`settings-tab-${t.id}`}
                class="tab"
                class:secret={t.id === 'threads'}
                aria-selected={tab === t.id}
                aria-controls="settings-pane"
                tabindex={tab === t.id ? 0 : -1}
                onclick={() => (tab = t.id)}
                onkeydown={tabKey}
              >
                <span class="mark" aria-hidden="true">{t.mark}</span>
                <span class="tab-label">{t.label}</span>
                {#if changed(t.id) > 0}<span class="dot" title={`${changed(t.id)} changed from the default`}></span>{/if}
              </button>
            </li>
          {/each}
        </ul>
      </nav>

      <div class="pane" id="settings-pane" role="tabpanel" aria-labelledby={`settings-tab-${tab}`}>
        {#if tab === 'appearance'}
          <h3>Appearance</h3>
          <p class="lead">How the app looks. A theme is shown as soon as it is picked; it is kept when you save.</p>
          <div class="row tall">
            <div class="about"><span class="title">Theme</span></div>
          </div>
          {#each themeGroups as group (group)}
            <div class="theme-group">
              <span class="muted group-name">{group}</span>
              <div class="themes" role="radiogroup" aria-label={`${group} themes`}>
                {#each themes.filter((t) => t.group === group) as theme (theme.value)}
                  <button type="button" role="radio" class="theme" aria-checked={draft.theme === theme.value} title={theme.note} onclick={() => (draft.theme = theme.value)}>
                    {theme.label}
                  </button>
                {/each}
              </div>
            </div>
          {/each}
          <p class="muted note">{themes.find((t) => t.value === draft.theme)?.note}</p>
          <div class="row">
            <div class="about"><span class="title">Size of text and controls</span></div>
            <select aria-label="Size of text and controls" bind:value={draft.zoom}>
              {#each [80, 90, 100, 110, 125, 150, 175] as percent}
                <option value={percent}>{percent}%</option>
              {/each}
            </select>
          </div>
        {:else if tab === 'editor'}
          <h3>Editor</h3>
          <p class="lead">Typing SQL, and what Reformat does to it.</p>
          <div class="row">
            <div class="about"><span class="title">Indent</span></div>
            <select aria-label="Indent" bind:value={draft.editorTabSize}>
              <option value={2}>2 spaces</option>
              <option value={4}>4 spaces</option>
              <option value={8}>8 spaces</option>
            </select>
          </div>
          <label class="row">
            <div class="about"><span class="title">Wrap long lines</span></div>
            <input type="checkbox" bind:checked={draft.editorWordWrap} />
          </label>
          <label class="row">
            <div class="about">
              <span class="title">Keywords in UPPER CASE when completing</span>
              <span class="muted note">What the list of suggestions offers as you type.</span>
            </div>
            <input type="checkbox" bind:checked={draft.uppercaseKeywords} />
          </label>
          <h4>Reformat</h4>
          <div class="row">
            <div class="about">
              <span class="title">Layout</span>
              <span class="muted note">{REFORMAT_STYLES.find((s) => s.value === draft.reformatStyle)?.note}</span>
            </div>
            <select aria-label="Reformat layout" bind:value={draft.reformatStyle}>
              {#each REFORMAT_STYLES as style (style.value)}<option value={style.value}>{style.label}</option>{/each}
            </select>
          </div>
          <pre class="sample">{REFORMAT_STYLES.find((s) => s.value === draft.reformatStyle)?.sample}</pre>
          <div class="row">
            <div class="about"><span class="title">Keywords</span></div>
            <select aria-label="Reformat keywords" bind:value={draft.reformatKeywords}>
              <option value="upper">IN UPPER CASE</option>
              <option value="lower">in lower case</option>
              <option value="preserve">As they were typed</option>
            </select>
          </div>
        {:else if tab === 'queries'}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
          <h3 onclick={knock}>Queries</h3>
          <p class="lead">Limits on what a query may take: rows, time and memory.</p>
          <div class="row">
            <div class="about">
              <span class="title">Row limit for a new connection</span>
              <span class="muted note">At the limit the query is stopped on the server. It can be changed per connection in the toolbar.</span>
            </div>
            <select aria-label="Row limit for a new connection" bind:value={draft.defaultRowLimit}>
              {#each rowLimits as option}
                <option value={option.value}>{option.label}</option>
              {/each}
            </select>
          </div>
          <div class="row">
            <div class="about">
              <span class="title">Stop a query after</span>
              <span class="muted note">
                0 means never: a query runs until it finishes or Stop is pressed. Otherwise a statement that runs longer is interrupted on
                the server, and the rest of the script is not run. For a query run In Slices, it applies to each slice.
              </span>
            </div>
            <span class="with-unit"><input type="number" aria-label="Stop a query after, in seconds" min="0" max="86400" step="1" bind:value={draft.queryTimeoutSecs} /> s</span>
          </div>
          <div class="row">
            <div class="about">
              <span class="title">Memory for results before using disk</span>
              <span class="muted note">Shared by all open results. Beyond it, rows go to a temporary file that is removed when the result closes.</span>
            </div>
            <span class="with-unit"><input type="number" aria-label="Memory for results, in megabytes" min="64" max="16384" step="64" bind:value={draft.resultMemoryMb} /> MB</span>
          </div>
        {:else if tab === 'connections'}
          <h3>Connections</h3>
          <p class="lead">The Connections page, and how connections to a server are opened and kept.</p>
          <label class="row">
            <div class="about">
              <span class="title">Organise connections by dragging</span>
              <span class="muted note">
                Reorder them and gather them into named groups. Off, the Connections page is a plain list and nothing on it can be dragged;
                groups already made are kept.
              </span>
            </div>
            <input type="checkbox" bind:checked={draft.organizeConnections} />
          </label>
          <div class="row">
            <div class="about">
              <span class="title">Import from MySQL Workbench</span>
              <span class="muted note">Reads the connections Workbench has saved and brings over the ones you tick. Passwords are not imported.</span>
            </div>
            <button type="button" onclick={importConnections}>Import&hellip;</button>
          </div>
          <h4>Timing</h4>
          <div class="row">
            <div class="about"><span class="title">Give up connecting after</span></div>
            <span class="with-unit"><input type="number" aria-label="Give up connecting after, in seconds" min="3" max="300" step="1" bind:value={draft.connectTimeoutSecs} /> s</span>
          </div>
          <div class="row">
            <div class="about">
              <span class="title">Keep idle connections alive every</span>
              <span class="muted note">
                0 means off. Servers close connections that sit idle; a closed one is reopened the next time it is used, but session
                variables and temporary tables are lost. A keep-alive of a few minutes prevents that, at the cost of holding the
                connections open for as long as the app is.
              </span>
            </div>
            <span class="with-unit"><input type="number" aria-label="Keep idle connections alive every, in seconds" min="0" max="3600" step="30" bind:value={draft.keepAliveSecs} /> s</span>
          </div>
        {:else if tab === 'privacy'}
          <h3>Privacy</h3>
          <p class="lead">What is kept on this computer between sessions. Neither is encrypted; passwords are never among it.</p>
          <label class="row">
            <div class="about">
              <span class="title">Keep a history of executed statements</span>
              <span class="muted note">
                Statements that contain a password are never stored in full. Turning this off stops recording; Clear in the History
                window removes what is already there.
              </span>
            </div>
            <input type="checkbox" bind:checked={draft.keepHistory} />
          </label>
          <label class="row">
            <div class="about">
              <span class="title">Bring back open query tabs</span>
              <span class="muted note">
                When a connection is opened again, its query tabs come back with their text. A tab holding a statement with a password is
                never kept.
              </span>
            </div>
            <input type="checkbox" bind:checked={draft.rememberTabs} />
          </label>
        {:else if tab === 'threads'}
          <h3 class="secret-title"><span aria-hidden="true">&#8649;</span> Threadripper</h3>
          <p class="lead">
            You found it. A query run with <strong>In Slices</strong> normally runs its slices one after another. Here it can run several at
            once, each on a connection of its own, and still put the rows together in order.
          </p>
          <div class="row tall">
            <div class="about">
              <span class="title">Slices at once</span>
              <span class="muted note">
                More is faster until the server is the limit. Every one is a real connection carrying a real query, so go easy on a server
                other people use.
              </span>
            </div>
          </div>
          <div class="threads" role="radiogroup" aria-label="Slices run at once">
            {#each THREADS as count (count)}
              <button type="button" role="radio" aria-checked={draft.sliceThreads === count} onclick={() => (draft.sliceThreads = count)}>{count}</button>
            {/each}
            <span class="muted">{draft.sliceThreads === 1 ? 'one at a time' : `${draft.sliceThreads} slices at a time`}</span>
          </div>
        {/if}
      </div>
    </div>

    {#if error}<p class="error-text" role="alert">{error}</p>{/if}

    <div class="buttons">
      <button type="button" disabled={changed(tab) === 0} title="Put the settings on this tab back to their defaults" onclick={restoreTab}>Restore this tab's defaults</button>
      <span class="spacer"></span>
      <button type="button" onclick={cancel}>Cancel</button>
      <button type="submit" class="primary" disabled={saving}>{saving ? 'Saving…' : 'Save'}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(780px, calc(100vw - 32px));
    max-width: min(780px, calc(100vw - 32px));
  }

  form {
    display: flex;
    flex-direction: column;
  }

  /* The same height whichever tab is in front, so the window does not jump about. */
  .frame {
    display: grid;
    grid-template-columns: 184px minmax(0, 1fr);
    height: min(500px, calc(100vh - 150px));
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px 10px 12px;
    background: var(--panel);
    border-right: 1px solid var(--border);
    border-radius: 8px 0 0 0;
  }

  h2 {
    margin: 0 8px;
    font-size: 16px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 10px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    text-align: left;
    color: var(--text);
  }

  .tab:hover {
    background: var(--panel-2);
  }

  .tab[aria-selected='true'] {
    background: var(--bg);
    border-color: var(--border);
    font-weight: 600;
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .tab.secret {
    color: var(--accent);
  }

  .mark {
    width: 16px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }

  .tab[aria-selected='true'] .mark,
  .tab.secret .mark {
    color: var(--accent);
  }

  .tab-label {
    flex: 1;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }

  .pane {
    overflow-y: auto;
    padding: 18px 22px 16px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  /* The pane scrolls; nothing in it is squeezed to fit. */
  .pane > :global(*) {
    flex: none;
  }

  h3 {
    margin: 0;
    font-size: 17px;
    font-weight: 650;
  }

  .secret-title {
    color: var(--accent);
  }

  h4 {
    margin: 16px 0 0;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  .lead {
    margin: 2px 0 8px;
    color: var(--muted);
  }

  /* A setting: what it is on the left, the control on the right. */
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 9px 0;
    border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  }

  .row.tall {
    padding-bottom: 4px;
  }

  label.row {
    cursor: pointer;
  }

  .about {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title {
    font-weight: 600;
  }

  .note {
    font-size: 12px;
  }

  .row select,
  .row input[type='number'] {
    flex: none;
    width: 150px;
  }

  .with-unit {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
  }

  .with-unit input[type='number'] {
    width: 110px;
  }

  .row input[type='checkbox'] {
    flex: none;
    width: 16px;
    height: 16px;
  }

  .theme-group {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 3px 0;
  }

  .group-name {
    width: 70px;
    flex: none;
    font-size: 12px;
  }

  .themes,
  .threads {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
  }

  .theme {
    padding: 3px 11px;
    border-radius: 999px;
  }

  .theme[aria-checked='true'],
  .threads button[aria-checked='true'] {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
  }

  .threads button {
    min-width: 38px;
    padding: 4px 0;
  }

  .threads span {
    margin-left: 8px;
  }

  .sample {
    margin: 0 0 6px;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.45;
    white-space: pre;
    overflow-x: auto;
  }

  p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .error-text {
    padding: 8px 20px 0;
  }

  .buttons {
    display: flex;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--border);
  }

  .spacer {
    flex: 1;
  }
</style>
