<script lang="ts">
  import { adminQueries } from '../lib/admin';
  import { backend } from '../lib/backend';
  import { separator, type MenuItem } from '../lib/menu';
  import { app, forgetSavedTabs, type WorkspaceState } from '../lib/state.svelte';
  import { asUiError, explainConnectError, newProfile, type Profile, type TlsSetting } from '../lib/types';
  import ConnectionDialog from './ConnectionDialog.svelte';
  import Menu from './Menu.svelte';
  import PasswordPrompt from './PasswordPrompt.svelte';

  let editing = $state<Profile | null>(null);
  let asking = $state<Profile | null>(null);
  let promptError = $state<string | null>(null);
  let busyId = $state<string | null>(null);
  let errors = $state<Record<string, string>>({});
  let demoError = $state<string | null>(null);
  /** The menu behind a connection's "more" button. */
  let menu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null);
  /** The connection being asked about before it is deleted. */
  let deleting = $state<Profile | null>(null);
  let deleteError = $state<string | null>(null);
  /** The connection whose server information is being chosen from. */
  let infoFor = $state<Profile | null>(null);
  /** A server-information query to run once its connection is open (it may need a password first). */
  let afterConnect: { profileId: string; title: string; sql: string } | null = null;

  const label = (profile: Profile) => profile.name || `${profile.user}@${profile.host}`;

  function modal(node: HTMLDialogElement) {
    node.showModal();
  }

  /** Everything about a connection that its card leaves out, for the tooltip. */
  const more = (profile: Profile) =>
    [
      `${profile.user}@${profile.host}:${profile.port}`,
      tlsLabels[profile.tls],
      profile.database ? `default schema ${profile.database}` : '',
      profile.cleartextAuth ? 'cleartext auth' : '',
      profile.hasPassword ? 'password in the keychain' : 'asks for the password',
    ]
      .filter(Boolean)
      .join(' \u00b7 ');

  const tlsLabels: Record<TlsSetting, string> = {
    'verify-identity': 'Encrypted, verified',
    'verify-ca': 'Encrypted, CA verified',
    pinned: 'Encrypted, pinned certificate',
    unverified: 'Encrypted, not verified',
    disabled: 'Not encrypted',
  };

  async function connect(profile: Profile, password: string | null = null, remember = false) {
    busyId = profile.id;
    delete errors[profile.id];
    try {
      const info = await backend.connect(profile.id, password);
      if (remember && password !== null) {
        await backend.profileSave({ ...profile, savePassword: true }, password);
        await app.refreshProfiles();
      }
      asking = null;
      promptError = null;
      const opened = app.open(info);
      if (afterConnect?.profileId === profile.id) {
        runIn(opened, afterConnect.title, afterConnect.sql);
        afterConnect = null;
      }
    } catch (e) {
      const error = asUiError(e);
      if (error.kind === 'password-required') {
        asking = profile;
        promptError = null;
      } else if (asking && error.kind === 'access-denied') {
        promptError = error.message;
      } else {
        asking = null;
        afterConnect = null;
        errors[profile.id] = explainConnectError(error);
      }
    } finally {
      busyId = null;
    }
  }

  async function connectDemo() {
    demoError = null;
    busyId = 'demo';
    try {
      app.open(await backend.connectDemo());
    } catch (e) {
      demoError = asUiError(e).message;
    } finally {
      busyId = null;
    }
  }

  async function remove(profile: Profile) {
    deleteError = null;
    try {
      await backend.profileDelete(profile.id);
      forgetSavedTabs(profile.id);
      deleting = null;
      await app.refreshProfiles();
    } catch (e) {
      deleteError = asUiError(e).message;
    }
  }

  function showMenu(e: MouseEvent, profile: Profile) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      items: [
        { label: 'Edit…', action: () => (editing = { ...profile }) },
        { label: 'Server Information…', action: () => (infoFor = profile) },
        separator,
        { label: 'Delete…', action: () => (deleting = profile) },
      ],
      x: rect.right - 200,
      y: rect.bottom + 4,
    };
  }

  /** Opens a tab in the connection with the statement in it, and runs it. */
  function runIn(workspace: WorkspaceState, title: string, sql: string) {
    app.active = workspace.info.id;
    const tab = workspace.addTab(sql, title);
    void workspace.run(tab, sql, { kind: 'all' });
  }

  /** Runs a server-information query in the connection, connecting first if it is not open. */
  function showServerInfo(profile: Profile, item: { title: string; sql: string }) {
    infoFor = null;
    const open = app.workspaces.find((w) => w.info.profileId === profile.id);
    if (open) {
      runIn(open, item.title, item.sql);
      return;
    }
    afterConnect = { profileId: profile.id, title: item.title, sql: item.sql };
    void connect(profile);
  }
</script>

<main class="home">
  <header>
    <div>
      <h1>Connections</h1>
      {#if app.profiles.length > 0}
        <p class="muted count">{app.profiles.length} saved</p>
      {/if}
    </div>
    <button class="primary" onclick={() => (editing = newProfile())}>+ New connection</button>
  </header>

  {#if app.profiles.length === 0}
    <div class="empty">
      <div class="empty-mark" aria-hidden="true">&#9783;</div>
      <p class="empty-title">No connections yet</p>
      <p class="muted">
        Create one to connect to a MySQL or MariaDB server. If you use MySQL Workbench, its connections can be brought over from
        Settings.
      </p>
      <div class="empty-buttons">
        <button class="primary" onclick={() => (editing = newProfile())}>+ New connection</button>
        <button onclick={() => (app.showSettings = true)}>Settings</button>
      </div>
    </div>
  {/if}

  <ul class="profiles">
    {#each app.profiles as profile (profile.id)}
      {@const open = app.workspaces.some((w) => w.info.profileId === profile.id)}
      <li class="card" class:failed={!!errors[profile.id]} style:--tag={profile.color ? `var(--tag-${profile.color})` : 'var(--border)'} title={more(profile)}>
        <div class="card-top">
          <span class="avatar" aria-hidden="true">{(label(profile).trim()[0] ?? '?').toUpperCase()}</span>
          <div class="who">
            <div class="name">{label(profile)}</div>
            <div class="muted where">{profile.user}@{profile.host}</div>
          </div>
          <button
            class="quiet more"
            aria-label={`More for ${label(profile)}`}
            aria-haspopup="menu"
            title="Edit, server information, delete"
            onclick={(e) => showMenu(e, profile)}>&#9776;</button
          >
        </div>
        {#if errors[profile.id]}
          <p class="error-text" role="alert">{errors[profile.id]}</p>
        {/if}
        <div class="card-foot">
          <!-- The two that change what a connection will let you do stay in sight. -->
          {#if profile.production}<span class="pill production" title="Risky statements ask for confirmation">Production</span>{/if}
          {#if profile.readOnly}<span class="pill" title="Only reads are allowed on this connection">Read-only</span>{/if}
          {#if open}<span class="pill open" title="This connection is open in a tab">Open</span>{/if}
          <span class="spacer"></span>
          {#if errors[profile.id]}
            <button onclick={() => (editing = { ...profile })}>Edit settings</button>
          {/if}
          <button class="primary" disabled={busyId !== null} onclick={() => connect(profile)}>
            {busyId === profile.id ? 'Connecting\u2026' : 'Connect'}
          </button>
        </div>
      </li>
    {/each}
  </ul>

  {#if app.info?.devtools}
    <section class="demo">
      <div>
        <div class="name">Demo server</div>
        <div class="muted">
          A fake MySQL Enterprise server built into development builds. Try
          <code>SELECT fake_rows(1000000, 20)</code>.
        </div>
        {#if demoError}<p class="error-text" role="alert">{demoError}</p>{/if}
      </div>
      <button disabled={busyId !== null} onclick={connectDemo}>
        {busyId === 'demo' ? 'Connecting…' : 'Open demo'}
      </button>
    </section>
  {/if}

  {#if app.info}
    <p class="version muted">SQL Tool {app.info.version}</p>
  {/if}
</main>

{#if menu}
  <Menu items={menu.items} label="Connection" x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

{#if deleting}
  {@const profile = deleting}
  <dialog use:modal aria-labelledby="delete-title" onclose={() => (deleting = null)}>
    <div class="prompt">
      <h2 id="delete-title">Delete the connection {label(profile)}?</h2>
      <p class="muted">
        This removes the saved connection, its password from the keychain, and the query tabs and default schema remembered for it.
        Nothing on the server changes.
      </p>
      {#if deleteError}<p class="error-text" role="alert">{deleteError}</p>{/if}
      <div class="buttons">
        <button onclick={() => (deleting = null)}>Cancel</button>
        <button class="danger" onclick={() => remove(profile)}>Delete</button>
      </div>
    </div>
  </dialog>
{/if}

{#if infoFor}
  {@const profile = infoFor}
  <dialog use:modal aria-labelledby="info-title" onclose={() => (infoFor = null)}>
    <div class="prompt wide">
      <h2 id="info-title">Server information</h2>
      <p class="muted">
        {label(profile)}. Each of these is an ordinary query: it opens in a tab of the connection and runs there, so you can see
        and change it.
        {#if !app.workspaces.some((w) => w.info.profileId === profile.id)}The connection is opened first.{/if}
      </p>
      <div class="choices">
        {#each adminQueries as item (item.title)}
          <button class="choice" onclick={() => showServerInfo(profile, item)}>
            <span class="choice-title">{item.title}</span>
            <span class="muted">{item.note}</span>
            <code>{item.sql}</code>
          </button>
        {/each}
      </div>
      <div class="buttons">
        <button onclick={() => (infoFor = null)}>Close</button>
      </div>
    </div>
  </dialog>
{/if}

{#if editing}
  <ConnectionDialog
    profile={editing}
    onclose={() => (editing = null)}
    onsaved={async () => {
      editing = null;
      await app.refreshProfiles();
    }}
  />
{/if}

{#if asking}
  {@const profile = asking}
  <PasswordPrompt
    title={profile.name || `${profile.user}@${profile.host}`}
    user={profile.user}
    error={promptError}
    busy={busyId === profile.id}
    oncancel={() => (asking = null)}
    onsubmit={(password, remember) => connect(profile, password, remember)}
  />
{/if}

<style>
  .home {
    flex: 1;
    overflow: auto;
    padding: 28px max(24px, calc((100% - 920px) / 2)) 24px;
    background: var(--panel);
  }

  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: 18px;
  }

  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .count {
    margin: 2px 0 0;
    font-size: 12px;
  }

  /* Two to a row; one when the window is too narrow for two to read well. */
  .profiles {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  @media (max-width: 720px) {
    .profiles {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px 14px 12px 16px;
    border: 1px solid var(--border);
    /* The connection's colour tag, as a band down the side. */
    border-left: 4px solid var(--tag);
    border-radius: 10px;
    background: var(--bg);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.05);
    transition:
      box-shadow 0.12s ease,
      transform 0.12s ease,
      border-color 0.12s ease;
  }

  .card:hover {
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.1);
    transform: translateY(-1px);
    border-top-color: var(--accent);
    border-right-color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .card.failed {
    border-top-color: var(--danger);
    border-right-color: var(--danger);
    border-bottom-color: var(--danger);
  }

  .card-top {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .avatar {
    width: 38px;
    height: 38px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 10px;
    background: color-mix(in srgb, var(--tag) 22%, var(--panel));
    color: var(--text);
    font-size: 17px;
    font-weight: 650;
  }

  .who {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: 600;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .where {
    font-family: var(--mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .more {
    flex: none;
    padding: 4px 9px;
    font-size: 15px;
  }

  .card-foot {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 28px;
  }

  .card-foot .primary {
    min-width: 96px;
  }

  .spacer {
    flex: 1;
  }

  .pill {
    padding: 1px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 11px;
    color: var(--muted);
    white-space: nowrap;
  }

  .pill.production {
    border-color: var(--warn-border);
    background: var(--warn-bg);
    color: var(--warn-text);
  }

  .pill.open {
    border-color: var(--ok);
    color: var(--ok);
  }

  .card .error-text {
    margin: 0;
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .demo {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  .demo .name {
    font-size: 14px;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 44px 24px;
    border: 1px dashed var(--border);
    border-radius: 12px;
    background: var(--bg);
    text-align: center;
  }

  .empty p {
    margin: 0;
    max-width: 420px;
  }

  .empty-mark {
    font-size: 30px;
    color: var(--muted);
    line-height: 1;
  }

  .empty-title {
    font-size: 16px;
    font-weight: 600;
  }

  .empty-buttons {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }

  .prompt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 20px;
    width: 440px;
    max-width: 100%;
  }

  .prompt.wide {
    width: 600px;
  }

  .prompt h2 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }

  .prompt p {
    margin: 0;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }

  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .choice {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--panel);
    text-align: left;
    white-space: normal;
  }

  .choice:hover {
    background: var(--selection);
  }

  .choice-title {
    font-weight: 600;
    font-size: 14px;
  }

  .choice code {
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .demo p {
    margin: 8px 0 0;
  }

  .demo {
    margin-top: 24px;
    justify-content: space-between;
    background: var(--bg);
    border-style: dashed;
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
  }

  .version {
    margin-top: 24px;
    font-size: 12px;
  }
</style>
