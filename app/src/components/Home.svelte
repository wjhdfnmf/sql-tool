<script lang="ts">
  import { adminQueries } from '../lib/admin';
  import { backend } from '../lib/backend';
  import { separator, type MenuItem } from '../lib/menu';
  import { settings } from '../lib/settings.svelte';
  import { app, forgetSavedTabs, type WorkspaceState } from '../lib/state.svelte';
  import { asUiError, explainConnectError, newProfile, type Profile, type TlsSetting } from '../lib/types';
  import ConnectionDialog from './ConnectionDialog.svelte';
  import Menu from './Menu.svelte';
  import PasswordPrompt from './PasswordPrompt.svelte';

  let editing = $state<Profile | null>(null);
  let asking = $state<Profile | null>(null);
  let promptError = $state<string | null>(null);
  /** Something to tell about the keychain after a save that went through without it. */
  let notice = $state<string | null>(null);
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

  // ---------- order and groups ----------
  //
  // The order of the connections is the order they are stored in, and a
  // group is a name that some of them carry. Every change here makes a new
  // list of (connection, group) in the order wanted and has the backend store it.

  interface Section {
    /** Null for the connections that are in no group, which come first. */
    name: string | null;
    profiles: Profile[];
  }

  const organize = $derived(settings.current.organizeConnections);
  const sections = $derived.by((): Section[] => {
    if (!organize) return [{ name: null, profiles: app.profiles }];
    const loose: Section = { name: null, profiles: [] };
    const named: Section[] = [];
    for (const profile of app.profiles) {
      const name = profile.group?.trim() || null;
      const section = name === null ? loose : (named.find((g) => g.name === name) ?? (named.push({ name, profiles: [] }), named[named.length - 1]));
      section.profiles.push(profile);
    }
    return loose.profiles.length > 0 || named.length === 0 ? [loose, ...named] : named;
  });
  const groupNames = $derived(sections.flatMap((section) => (section.name === null ? [] : [section.name])));

  let arrangeError = $state<string | null>(null);

  /** Stores an arrangement: the page shows it at once, and the stored list replaces it when it comes back. */
  async function arrange(next: Section[]) {
    const flat = next.flatMap((section) => section.profiles.map((profile) => ({ ...profile, group: section.name })));
    const before = app.profiles;
    app.profiles = flat;
    arrangeError = null;
    try {
      app.profiles = await backend.profilesArrange(flat.map((profile) => ({ id: profile.id, group: profile.group ?? null })));
    } catch (e) {
      app.profiles = before;
      arrangeError = `The new arrangement could not be saved: ${asUiError(e).message}`;
    }
  }

  /** The sections as they are, as plain copies to rearrange. */
  const copy = (): Section[] => sections.map((section) => ({ name: section.name, profiles: [...section.profiles] }));

  /** Moves a connection into a group (null for none), before another connection or at the end. */
  function moveCard(id: string, group: string | null, beforeId: string | null) {
    if (id === beforeId) return;
    const next = copy();
    const moved = next.flatMap((section) => section.profiles).find((profile) => profile.id === id);
    if (!moved) return;
    for (const section of next) section.profiles = section.profiles.filter((profile) => profile.id !== id);
    let target = next.find((section) => section.name === group);
    if (!target) {
      target = { name: group, profiles: [] };
      // Connections in no group come first; a new group goes last.
      if (group === null) next.unshift(target);
      else next.push(target);
    }
    const at = beforeId === null ? -1 : target.profiles.findIndex((profile) => profile.id === beforeId);
    target.profiles.splice(at < 0 ? target.profiles.length : at, 0, moved);
    void arrange(next);
  }

  /** Moves a connection one place earlier or later within its group. */
  function nudge(profile: Profile, by: -1 | 1) {
    const section = sections.find((s) => s.profiles.some((p) => p.id === profile.id));
    if (!section) return;
    const at = section.profiles.findIndex((p) => p.id === profile.id);
    const to = at + by;
    if (to < 0 || to >= section.profiles.length) return;
    // Later means after the next one: before the one after that, or at the end.
    moveCard(profile.id, section.name, by < 0 ? section.profiles[to].id : (section.profiles[to + 1]?.id ?? null));
  }

  /** A group name not in use yet: "New group", "New group 2", and so on. */
  function freeGroupName(): string {
    for (let n = 1; ; n++) {
      const name = n === 1 ? 'New group' : `New group ${n}`;
      if (!groupNames.includes(name)) return name;
    }
  }

  /** Starts a group with one connection in it, and opens its name for typing. */
  function startGroup(id: string) {
    const name = freeGroupName();
    moveCard(id, name, null);
    renaming = { from: name, text: name };
  }

  /** The group whose name is being typed. */
  let renaming = $state<{ from: string; text: string } | null>(null);

  function finishRename(keep: boolean) {
    const current = renaming;
    renaming = null;
    if (!current || !keep) return;
    const name = current.text.trim().slice(0, 60);
    if (name === '' || name === current.from) return;
    // Given the name of another group, the two become one.
    const next = copy();
    const from = next.find((section) => section.name === current.from);
    const into = next.find((section) => section.name === name);
    if (!from) return;
    if (into) {
      into.profiles.push(...from.profiles);
      void arrange(next.filter((section) => section !== from));
    } else {
      from.name = name;
      void arrange(next);
    }
  }

  /** Dissolves a group: its connections go back among those in no group. */
  function ungroup(name: string) {
    const next = copy();
    const from = next.find((section) => section.name === name);
    if (!from) return;
    let loose = next.find((section) => section.name === null);
    if (!loose) {
      loose = { name: null, profiles: [] };
      next.unshift(loose);
    }
    loose.profiles.push(...from.profiles);
    void arrange(next.filter((section) => section !== from));
  }

  function moveGroup(name: string, by: -1 | 1) {
    const next = copy();
    const at = next.findIndex((section) => section.name === name);
    const to = at + by;
    // The connections in no group stay first.
    if (at < 0 || to < 0 || to >= next.length || next[to].name === null) return;
    [next[at], next[to]] = [next[to], next[at]];
    void arrange(next);
  }

  // Which groups are folded away is a matter of this window, not of the connections: it is kept here, not with them.
  const FOLDED_KEY = 'sqltool.groups.folded';
  let folded = $state<string[]>(
    (() => {
      try {
        const kept: unknown = JSON.parse(localStorage.getItem(FOLDED_KEY) ?? '[]');
        return Array.isArray(kept) ? kept.filter((name): name is string => typeof name === 'string') : [];
      } catch {
        return [];
      }
    })(),
  );

  function fold(name: string) {
    folded = folded.includes(name) ? folded.filter((n) => n !== name) : [...folded, name];
    try {
      localStorage.setItem(FOLDED_KEY, JSON.stringify(folded));
    } catch {
      // Remembering what is folded is a convenience.
    }
  }

  // ---------- dragging ----------

  /** Where a dragged connection would land. */
  type Drop = { group: string | null; beforeId: string | null; side: 'before' | 'after' | 'end'; overId: string | null } | { fresh: true };
  let dragging = $state<{ id: string; x: number; y: number; drop: Drop | null } | null>(null);
  /** Set when a drag has just ended, so that the click that follows it does nothing. */
  let justDragged = false;

  /** What is under the pointer: a card (and which half of it), the empty part of a group, or the place for a new group. */
  function dropAt(x: number, y: number, id: string): Drop | null {
    const under = document.elementFromPoint(x, y);
    if (under?.closest('[data-fresh]')) return { fresh: true };
    const section = under?.closest<HTMLElement>('[data-section]');
    if (!section) return null;
    const group = section.dataset.section === '' ? null : (section.dataset.section ?? null);
    const card = under?.closest<HTMLElement>('[data-card]');
    const members = sections.find((s) => s.name === group)?.profiles ?? [];
    if (!card || card.dataset.card === id) return card ? null : { group, beforeId: null, side: 'end', overId: null };
    const box = card.getBoundingClientRect();
    const overId = card.dataset.card ?? null;
    if (x < box.left + box.width / 2) return { group, beforeId: overId, side: 'before', overId };
    const at = members.findIndex((p) => p.id === overId);
    // After this card is before the next one that is not the card being dragged.
    const next = members.slice(at + 1).find((p) => p.id !== id);
    return { group, beforeId: next?.id ?? null, side: 'after', overId };
  }

  function startDrag(e: PointerEvent, profile: Profile) {
    // Not from a button on the card, and only with the main button.
    if (!organize || e.button !== 0 || (e.target as HTMLElement).closest('button, input, a')) return;
    const [startX, startY] = [e.clientX, e.clientY];
    let moved = false;
    const move = (ev: PointerEvent) => {
      if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 6) return;
      moved = true;
      dragging = { id: profile.id, x: ev.clientX, y: ev.clientY, drop: dropAt(ev.clientX, ev.clientY, profile.id) };
    };
    const stop = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      const drop = ev.type === 'pointerup' && moved ? dropAt(ev.clientX, ev.clientY, profile.id) : null;
      dragging = null;
      if (!moved) return;
      justDragged = true;
      setTimeout(() => (justDragged = false));
      if (!drop) return;
      if ('fresh' in drop) startGroup(profile.id);
      else moveCard(profile.id, drop.group, drop.beforeId);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  }

  /** The mark a card shows while something is dragged over it: a bar on the side the dragged card would go. */
  const dropSide = (profile: Profile) => (dragging?.drop && !('fresh' in dragging.drop) && dragging.drop.overId === profile.id ? dragging.drop.side : null);

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
        const saved = await backend.profileSave({ ...profile, savePassword: true }, password);
        if (saved.keychainNote) notice = saved.keychainNote;
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
      if (error.kind === 'password-required' || error.kind === 'keychain-unreadable') {
        asking = profile;
        // With no keychain to read, the password is asked for, and the prompt says why.
        promptError = error.kind === 'keychain-unreadable' ? error.message : null;
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
        // Everything dragging does can be done from here too.
        ...(organize
          ? [
              separator,
              { label: 'Move earlier', action: () => nudge(profile, -1) },
              { label: 'Move later', action: () => nudge(profile, 1) },
              separator,
              ...groupNames.map((name) => ({ label: `Move to \u201c${name}\u201d`, checked: (profile.group ?? null) === name, action: () => moveCard(profile.id, name, null) })),
              { label: 'Move to a new group…', action: () => startGroup(profile.id) },
              { label: 'Take out of its group', disabled: !profile.group, action: () => moveCard(profile.id, null, null) },
            ]
          : []),
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

  {#if notice}
    <div class="warning-box notice" role="status">
      <span>{notice}</span>
      <button onclick={() => (notice = null)}>Dismiss</button>
    </div>
  {/if}

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

{#snippet connection(profile: Profile)}
      {@const open = app.workspaces.some((w) => w.info.profileId === profile.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li
        class="card"
        class:failed={!!errors[profile.id]}
        class:movable={organize}
        class:lifted={dragging?.id === profile.id}
        class:drop-before={dropSide(profile) === 'before'}
        class:drop-after={dropSide(profile) === 'after'}
        data-card={profile.id}
        onpointerdown={(e) => startDrag(e, profile)} style:--tag={profile.color ? `var(--tag-${profile.color})` : 'var(--border)'} title={more(profile)}>
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
          <button class="primary" disabled={busyId !== null} onclick={() => !justDragged && connect(profile)}>
            {busyId === profile.id ? 'Connecting\u2026' : 'Connect'}
          </button>
        </div>
      </li>
{/snippet}

  {#if arrangeError}<p class="error-text" role="alert">{arrangeError}</p>{/if}

  {#each sections as section (section.name ?? '')}
    {@const name = section.name}
    {@const shut = name !== null && folded.includes(name)}
    <section class="group" class:target={dragging?.drop && !('fresh' in dragging.drop) && dragging.drop.group === name} data-section={name ?? ''}>
      {#if name !== null}
        <div class="group-head">
          <button class="quiet fold" aria-expanded={!shut} aria-label={`${shut ? 'Show' : 'Hide'} the connections in ${name}`} title={shut ? 'Show' : 'Hide'} onclick={() => fold(name)}>
            {shut ? '\u25B8' : '\u25BE'}
          </button>
          {#if renaming?.from === name}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="group-name-input"
              type="text"
              maxlength="60"
              aria-label="Name of the group"
              autofocus
              value={renaming?.text ?? ''}
              oninput={(e) => renaming && (renaming.text = e.currentTarget.value)}
              onfocus={(e) => e.currentTarget.select()}
              onblur={() => finishRename(true)}
              onkeydown={(e) => {
                if (e.key === 'Enter') finishRename(true);
                else if (e.key === 'Escape') {
                  e.stopPropagation();
                  finishRename(false);
                }
              }}
            />
          {:else}
            <button class="quiet group-name" title="Rename this group" onclick={() => (renaming = { from: name, text: name })}>{name}</button>
          {/if}
          <span class="muted group-count">{section.profiles.length}</span>
          <span class="rule"></span>
          <span class="group-tools">
            <button class="quiet" title="Move this group up" aria-label={`Move ${name} up`} disabled={groupNames.indexOf(name) === 0} onclick={() => moveGroup(name, -1)}>&#9652;</button>
            <button class="quiet" title="Move this group down" aria-label={`Move ${name} down`} disabled={groupNames.indexOf(name) === groupNames.length - 1} onclick={() => moveGroup(name, 1)}>&#9662;</button>
            <button class="quiet" title="Dissolve this group: its connections stay, in no group" aria-label={`Dissolve the group ${name}`} onclick={() => ungroup(name)}>&#215;</button>
          </span>
        </div>
      {:else if organize && groupNames.length > 0 && section.profiles.length > 0}
        <div class="group-head"><span class="group-name plain">Not in a group</span><span class="muted group-count">{section.profiles.length}</span><span class="rule"></span></div>
      {/if}
      {#if !shut}
        <ul class="profiles">
          {#each section.profiles as profile (profile.id)}
            {@render connection(profile)}
          {/each}
        </ul>
      {/if}
    </section>
  {/each}

  {#if dragging}
    <div class="fresh" class:target={dragging.drop && 'fresh' in dragging.drop} data-fresh>Drop here to start a new group</div>
    <div class="ghost" style:left={`${dragging.x + 12}px`} style:top={`${dragging.y + 10}px`}>{label(app.profiles.find((p) => p.id === dragging?.id) ?? newProfile())}</div>
  {:else if organize && app.profiles.length > 1 && groupNames.length === 0}
    <p class="muted hint">Drag a connection to move it. Drag it to the bottom of the page to start a group.</p>
  {/if}


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
    onsaved={async (note) => {
      editing = null;
      notice = note;
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
  .notice {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin-bottom: 14px;
  }

  .notice span {
    flex: 1;
    overflow-wrap: anywhere;
  }

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

  .group + .group {
    margin-top: 18px;
  }

  .group-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 8px;
    min-height: 26px;
  }

  .group-name {
    padding: 1px 6px;
    font-size: 13px;
    font-weight: 650;
    letter-spacing: 0.01em;
  }

  .group-name.plain {
    padding-left: 0;
    color: var(--muted);
  }

  .group-name-input {
    width: 220px;
    padding: 1px 6px;
    font-size: 13px;
    font-weight: 650;
  }

  .group-count {
    font-size: 12px;
  }

  .rule {
    flex: 1;
    height: 1px;
    background: var(--border);
    margin: 0 6px;
  }

  .fold {
    padding: 0 5px;
    font-size: 12px;
  }

  /* The tools of a group stay out of the way until the group is pointed at or one of them has the focus. */
  .group-tools {
    display: flex;
    opacity: 0;
    transition: opacity 0.12s ease;
  }

  .group:hover .group-tools,
  .group-tools:focus-within {
    opacity: 1;
  }

  .group-tools button {
    padding: 0 6px;
  }

  .group.target .rule {
    background: var(--accent);
    height: 2px;
  }

  .card.movable {
    cursor: grab;
    touch-action: none;
    user-select: none;
  }

  .card.lifted {
    opacity: 0.35;
  }

  /* Where a dragged connection would land: a bar on that side of the card it is over. */
  .card.drop-before,
  .card.drop-after {
    position: relative;
  }

  .card.drop-before::after,
  .card.drop-after::after {
    content: '';
    position: absolute;
    top: 4px;
    bottom: 4px;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
  }

  .card.drop-before::after {
    left: -9px;
  }

  .card.drop-after::after {
    right: -8px;
  }

  .fresh {
    margin-top: 16px;
    padding: 18px;
    border: 2px dashed var(--border);
    border-radius: 10px;
    text-align: center;
    color: var(--muted);
  }

  .fresh.target {
    border-color: var(--accent);
    background: var(--selection);
    color: var(--text);
  }

  .ghost {
    position: fixed;
    z-index: 50;
    pointer-events: none;
    padding: 6px 12px;
    border: 1px solid var(--accent);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.25);
    font-weight: 600;
  }

  .hint {
    margin: 12px 0 0;
    font-size: 12px;
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
