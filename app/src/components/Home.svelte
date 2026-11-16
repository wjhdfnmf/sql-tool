<script lang="ts">
  import { backend } from '../lib/backend';
  import { app } from '../lib/state.svelte';
  import { asUiError, explainConnectError, newProfile, type Profile, type TlsSetting } from '../lib/types';
  import ConnectionDialog from './ConnectionDialog.svelte';
  import PasswordPrompt from './PasswordPrompt.svelte';

  let editing = $state<Profile | null>(null);
  let asking = $state<Profile | null>(null);
  let promptError = $state<string | null>(null);
  let busyId = $state<string | null>(null);
  let errors = $state<Record<string, string>>({});
  let confirmDelete = $state<string | null>(null);
  let demoError = $state<string | null>(null);

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
      app.open(info);
    } catch (e) {
      const error = asUiError(e);
      if (error.kind === 'password-required') {
        asking = profile;
        promptError = null;
      } else if (asking && error.kind === 'access-denied') {
        promptError = error.message;
      } else {
        asking = null;
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
    if (confirmDelete !== profile.id) {
      confirmDelete = profile.id;
      setTimeout(() => {
        if (confirmDelete === profile.id) confirmDelete = null;
      }, 4000);
      return;
    }
    confirmDelete = null;
    await backend.profileDelete(profile.id);
    await app.refreshProfiles();
  }
</script>

<main class="home">
  <header>
    <h1>Connections</h1>
    <button class="primary" onclick={() => (editing = newProfile())}>New connection</button>
  </header>

  {#if app.profiles.length === 0}
    <p class="empty muted">
      No saved connections yet. Create one to connect to a MySQL or MariaDB server.
    </p>
  {/if}

  <ul class="profiles">
    {#each app.profiles as profile (profile.id)}
      <li class="profile">
        <span class="tag-dot" style:background={profile.color ? `var(--tag-${profile.color})` : 'var(--border)'}
        ></span>
        <div class="details">
          <div class="name">{profile.name || `${profile.user}@${profile.host}`}</div>
          <div class="muted">
            {profile.user}@{profile.host}:{profile.port}
            &middot; {tlsLabels[profile.tls]}
            {#if profile.cleartextAuth}&middot; cleartext auth{/if}
            {#if profile.readOnly}&middot; read-only{/if}
            {#if profile.production}&middot; production{/if}
          </div>
          {#if errors[profile.id]}
            <p class="error-text" role="alert">{errors[profile.id]}</p>
            <p><button onclick={() => (editing = { ...profile })}>Edit connection settings</button></p>
          {/if}
        </div>
        <div class="actions">
          <button class="primary" disabled={busyId !== null} onclick={() => connect(profile)}>
            {busyId === profile.id ? 'Connecting…' : 'Connect'}
          </button>
          <button onclick={() => (editing = { ...profile })}>Edit</button>
          <button class="danger" onclick={() => remove(profile)}>
            {confirmDelete === profile.id ? 'Confirm delete' : 'Delete'}
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
    padding: 24px max(24px, calc((100% - 860px) / 2));
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }

  .profiles {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .profile,
  .demo {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }

  .profile .tag-dot {
    margin-top: 5px;
  }

  .details {
    flex: 1;
    min-width: 0;
  }

  .details .muted {
    overflow-wrap: anywhere;
  }

  .name {
    font-weight: 600;
    font-size: 14px;
  }

  .actions {
    display: flex;
    gap: 6px;
    flex: none;
  }

  .details p,
  .demo p {
    margin: 8px 0 0;
  }

  .demo {
    margin-top: 24px;
    justify-content: space-between;
    background: var(--panel);
    border-style: dashed;
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
  }

  .empty {
    padding: 24px 0;
  }

  .version {
    margin-top: 24px;
    font-size: 12px;
  }
</style>
