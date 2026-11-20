<script lang="ts">
  // Bringing connections over from MySQL Workbench: its saved connections
  // are listed, the user ticks the ones to bring, and those are saved here.

  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { app } from '../lib/state.svelte';
  import { asUiError, type TlsSetting, type WorkbenchConnection, type WorkbenchFile } from '../lib/types';

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let dialog: HTMLDialogElement;
  let file = $state.raw<WorkbenchFile | null>(null);
  /** Nothing has been looked for yet, or a look is under way. */
  let looking = $state(true);
  let error = $state('');
  /** Which connections are ticked, by their place in the file. */
  let ticked = $state<boolean[]>([]);
  let saving = $state(false);
  /** What an import came to. */
  let outcome = $state('');

  const TLS: Record<TlsSetting, string> = {
    disabled: 'not encrypted',
    unverified: 'encrypted, not verified',
    'verify-ca': 'encrypted, CA verified',
    'verify-identity': 'encrypted, server verified',
    pinned: 'encrypted, pinned',
  };

  /** A connection already here with the same host, port and user: bringing it over again would make a double. */
  const twin = (found: WorkbenchConnection) =>
    found.profile === null ? undefined : app.profiles.find((p) => p.host.toLowerCase() === found.profile?.host.toLowerCase() && p.port === found.profile.port && p.user === found.profile.user);

  const usable = $derived(file ? file.connections.filter((c) => c.profile !== null).length : 0);
  const chosen = $derived(ticked.filter(Boolean).length);

  onMount(() => {
    dialog.showModal();
    void look(false);
  });

  async function look(pick: boolean) {
    looking = true;
    error = '';
    try {
      const found = await backend.workbenchConnections(pick);
      // Cancelling the file dialog leaves what was listed before.
      if (found || !pick) show(found);
    } catch (e) {
      error = asUiError(e).message;
    } finally {
      looking = false;
    }
  }

  function show(found: WorkbenchFile | null) {
    file = found;
    outcome = '';
    // Everything that can come over is ticked, except what is here already.
    ticked = found ? found.connections.map((c) => c.profile !== null && !twin(c)) : [];
  }

  function tickAll(on: boolean) {
    if (file) ticked = file.connections.map((c) => on && c.profile !== null);
  }

  /** A name not already in use: "Plant", then "Plant (Workbench)", then "Plant (Workbench 2)". */
  function freeName(name: string, taken: Set<string>): string {
    if (!taken.has(name.toLowerCase())) return name;
    for (let n = 1; ; n++) {
      const candidate = `${name} (Workbench${n === 1 ? '' : ` ${n}`})`;
      if (!taken.has(candidate.toLowerCase())) return candidate;
    }
  }

  async function bring() {
    if (!file || chosen === 0) return;
    saving = true;
    error = '';
    const taken = new Set(app.profiles.map((p) => p.name.toLowerCase()));
    let done = 0;
    try {
      for (const [index, found] of file.connections.entries()) {
        if (!ticked[index] || !found.profile) continue;
        const name = freeName(found.profile.name, taken);
        taken.add(name.toLowerCase());
        // No password is brought over: the connection asks for it when it is first used.
        await backend.profileSave({ ...found.profile, id: '', name, hasPassword: false }, null);
        ticked[index] = false;
        done++;
      }
      outcome = `${done} ${done === 1 ? 'connection' : 'connections'} brought over. ${done === 1 ? 'It asks' : 'Each asks'} for its password the first time it is used.`;
    } catch (e) {
      error = `${done > 0 ? `${done} brought over, then: ` : ''}${asUiError(e).message}`;
    } finally {
      await app.refreshProfiles().catch(() => {});
      saving = false;
    }
  }
</script>

<dialog bind:this={dialog} onclose={onclose} aria-labelledby="wb-title">
  <div class="body">
    <h2 id="wb-title">Import from MySQL Workbench</h2>
    {#if file}
      <p class="muted from" title={file.path}>From {file.path}</p>
    {/if}

    {#if looking && !file}
      <p class="muted">Looking for Workbench's connections…</p>
    {:else if !file}
      <p>
        MySQL Workbench's connections were not found in the usual place on this computer. If you have its
        <code>connections.xml</code> (from this computer or another), choose it.
      </p>
    {:else if file.connections.length === 0}
      <p>That file holds no connections.</p>
    {:else}
      <div class="tools">
        <span>{usable} of {file.connections.length} can be brought over</span>
        <span class="spacer"></span>
        <button class="link" onclick={() => tickAll(true)}>Tick all</button>
        <button class="link" onclick={() => tickAll(false)}>Untick all</button>
      </div>
      <ul class="list">
        {#each file.connections as found, index (index)}
          {@const profile = found.profile}
          {@const same = twin(found)}
          <li class:off={!profile}>
            <label>
              <input type="checkbox" bind:checked={ticked[index]} disabled={!profile || saving} aria-label={`Import ${found.name}`} />
              <span class="what">
                <span class="name">{found.name}</span>
                {#if profile}
                  <span class="muted where">
                    {profile.user}@{profile.host}:{profile.port}{profile.database ? ` · ${profile.database}` : ''} · {TLS[profile.tls]}{profile.cleartextAuth
                      ? ' · cleartext auth'
                      : ''}
                  </span>
                {:else}
                  <span class="muted where">{found.method}</span>
                {/if}
                {#if found.problem}<span class="note problem">Cannot be brought over. {found.problem}</span>{/if}
                {#if same}<span class="note">Already here as &ldquo;{same.name}&rdquo;. Tick it to bring it over again under another name.</span>{/if}
                {#each found.notes as note}<span class="note">{note}</span>{/each}
              </span>
            </label>
          </li>
        {/each}
      </ul>
      <p class="muted">
        Passwords are not brought over: Workbench keeps them in the system's vault, and this app does not read it. Nothing in Workbench is
        changed.
      </p>
    {/if}

    {#if error}<p class="error-text" role="alert">{error}</p>{/if}
    {#if outcome}<p class="done" role="status">{outcome}</p>{/if}

    <div class="actions">
      <button disabled={looking || saving} title="Read a connections.xml from somewhere else" onclick={() => look(true)}>Choose a file&hellip;</button>
      <span class="spacer"></span>
      <button onclick={() => dialog.close()}>{outcome ? 'Close' : 'Cancel'}</button>
      <button class="primary" disabled={chosen === 0 || saving} onclick={bring}>
        {saving ? 'Importing…' : `Import ${chosen === 0 ? '' : chosen} ${chosen === 1 ? 'connection' : 'connections'}`}
      </button>
    </div>
  </div>
</dialog>

<style>
  .body {
    width: 620px;
    max-width: 100%;
    padding: 16px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
  }

  p {
    margin: 0;
  }

  .from {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
  }

  .tools {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 12px;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: min(46vh, 380px);
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  li + li {
    border-top: 1px solid var(--border);
  }

  li label {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 7px 10px;
    cursor: pointer;
  }

  li.off label {
    cursor: default;
  }

  li label:hover {
    background: var(--panel);
  }

  li input {
    margin-top: 3px;
  }

  .what {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .name {
    font-weight: 600;
  }

  li.off .name {
    color: var(--muted);
  }

  .where,
  .note {
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .note {
    color: var(--warn-text);
    background: var(--warn-bg);
    border-left: 2px solid var(--warn-border);
    padding: 1px 6px;
  }

  .note.problem {
    color: var(--muted);
    background: none;
    border-left-color: var(--border);
  }

  .done {
    color: var(--ok);
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }

  .spacer {
    flex: 1;
  }

  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    font-size: 12px;
  }
</style>
