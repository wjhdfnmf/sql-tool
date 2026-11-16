<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, serverLabel, type Profile, type TlsSetting } from '../lib/types';

  interface Props {
    profile: Profile;
    onclose: () => void;
    onsaved: () => void;
  }

  let { profile, onclose, onsaved }: Props = $props();

  let dialog: HTMLDialogElement;
  // A working copy taken once when the dialog opens: nothing is changed
  // until Save, and later changes to the prop are deliberately ignored.
  const initial = untrack(() => profile);
  let draft = $state<Profile>({ ...initial });
  let password = $state('');
  let database = $state(initial.database ?? '');
  let caFile = $state(initial.caFile ?? '');
  let saving = $state(false);
  let testing = $state(false);
  let testResult = $state<{ ok: boolean; text: string } | null>(null);
  let saveError = $state<string | null>(null);

  const tlsOptions: { value: TlsSetting; label: string; help: string }[] = [
    {
      value: 'verify-identity',
      label: 'Encrypted, server verified (recommended)',
      help: "Checks the server's certificate and that it matches the host name. Needs a certificate from a trusted authority, or the CA file below.",
    },
    {
      value: 'verify-ca',
      label: 'Encrypted, certificate authority verified',
      help: 'Meant to check the certificate against the CA file but not the host name. A known driver defect currently makes it check the host name too.',
    },
    {
      value: 'unverified',
      label: 'Encrypted, not verified',
      help: 'Stops eavesdropping, but not a server that impersonates yours. This is what most MySQL tools do by default.',
    },
    {
      value: 'disabled',
      label: 'Not encrypted',
      help: 'Everything, including the login exchange, crosses the network unprotected.',
    },
  ];

  const colors = [null, 'red', 'orange', 'green', 'blue', 'purple'];

  const tlsHelp = $derived(tlsOptions.find((o) => o.value === draft.tls)?.help ?? '');
  const verifies = $derived(draft.tls === 'verify-identity' || draft.tls === 'verify-ca');
  const needsRiskAck = $derived(draft.cleartextAuth && !verifies);
  const problem = $derived.by(() => {
    if (draft.host.trim() === '') return 'Enter the server host name or address.';
    if (draft.user.trim() === '') return 'Enter the user name.';
    if (!Number.isInteger(draft.port) || draft.port < 1 || draft.port > 65535) return 'Port must be between 1 and 65535.';
    if (needsRiskAck && !draft.cleartextRiskAccepted) return 'Accept the cleartext risk below, or choose a verified encryption mode.';
    return null;
  });

  onMount(() => dialog.showModal());

  function current(): Profile {
    return {
      ...draft,
      host: draft.host.trim(),
      user: draft.user.trim(),
      database: database.trim() || null,
      caFile: verifies && caFile.trim() ? caFile.trim() : null,
      cleartextRiskAccepted: needsRiskAck && draft.cleartextRiskAccepted,
    };
  }

  async function test() {
    testing = true;
    testResult = null;
    try {
      const info = await backend.connectionTest(current(), password === '' ? null : password);
      const tls = info.tlsVersion ? info.tlsVersion.replace('TLSv', 'TLS ') : 'not encrypted';
      testResult = { ok: true, text: `Connected to ${serverLabel(info.version)} as ${info.currentUser} (${tls}).` };
    } catch (e) {
      const error = asUiError(e);
      testResult = {
        ok: false,
        text: error.kind === 'password-required' ? 'Enter the password to test the connection.' : error.message,
      };
    } finally {
      testing = false;
    }
  }

  async function save() {
    saving = true;
    saveError = null;
    try {
      await backend.profileSave(current(), draft.savePassword && password !== '' ? password : null);
      onsaved();
    } catch (e) {
      saveError = asUiError(e).message;
    } finally {
      saving = false;
    }
  }
</script>

<dialog bind:this={dialog} oncancel={onclose} aria-labelledby="connection-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      if (!problem) void save();
    }}
  >
    <h2 id="connection-title">{profile.id ? 'Edit connection' : 'New connection'}</h2>

    <div class="grid">
      <label class="wide">
        <span>Name</span>
        <input type="text" bind:value={draft.name} placeholder="Shown in the connection list" />
      </label>
      <label class="host">
        <span>Host</span>
        <input type="text" bind:value={draft.host} placeholder="db.example.com" autocomplete="off" spellcheck="false" />
      </label>
      <label class="port">
        <span>Port</span>
        <input type="number" bind:value={draft.port} min="1" max="65535" />
      </label>
      <label>
        <span>User name</span>
        <input type="text" bind:value={draft.user} autocomplete="off" spellcheck="false" />
      </label>
      <label>
        <span>Password</span>
        <input
          type="password"
          bind:value={password}
          autocomplete="off"
          placeholder={profile.hasPassword ? 'Stored in keychain' : ''}
        />
      </label>
      <label class="wide check">
        <input type="checkbox" bind:checked={draft.savePassword} />
        Save the password in the system keychain (otherwise you are asked each time)
      </label>
      <label>
        <span>Default schema <span class="muted">(optional)</span></span>
        <input type="text" bind:value={database} autocomplete="off" spellcheck="false" />
      </label>
      <label>
        <span>Colour tag</span>
        <select bind:value={draft.color}>
          {#each colors as color}
            <option value={color}>{color ? color[0].toUpperCase() + color.slice(1) : 'None'}</option>
          {/each}
        </select>
      </label>
    </div>

    <fieldset>
      <legend>Security</legend>
      <label>
        <span>Encryption</span>
        <select bind:value={draft.tls}>
          {#each tlsOptions as option}
            <option value={option.value}>{option.label}</option>
          {/each}
        </select>
      </label>
      <p class="help muted">{tlsHelp}</p>

      {#if verifies}
        <label>
          <span>CA certificate file <span class="muted">(optional, PEM)</span></span>
          <input
            type="text"
            bind:value={caFile}
            placeholder="Leave empty to use the system's trusted authorities"
            autocomplete="off"
            spellcheck="false"
          />
        </label>
      {/if}

      <label class="check">
        <input type="checkbox" bind:checked={draft.cleartextAuth} />
        Allow cleartext authentication (LDAP and PAM accounts)
      </label>

      {#if needsRiskAck}
        <div class="warning-box" role="note">
          <p>
            Cleartext authentication sends your actual password to the server. Without server verification, a
            server impersonating yours could capture it.
            {#if draft.tls === 'disabled'}With encryption off, anyone on the network path could read it too.{/if}
          </p>
          <label class="check">
            <input type="checkbox" bind:checked={draft.cleartextRiskAccepted} />
            I accept this risk for this connection
          </label>
        </div>
      {/if}
    </fieldset>

    {#if testResult}
      <p class={testResult.ok ? 'ok-text' : 'error-text'} role="status">{testResult.text}</p>
    {/if}
    {#if saveError}<p class="error-text" role="alert">{saveError}</p>{/if}
    {#if problem}<p class="muted">{problem}</p>{/if}

    <div class="buttons">
      <button type="button" disabled={testing || problem !== null} onclick={test}>
        {testing ? 'Testing…' : 'Test connection'}
      </button>
      <span class="spacer"></span>
      <button type="button" onclick={onclose}>Cancel</button>
      <button type="submit" class="primary" disabled={saving || problem !== null}>Save</button>
    </div>
  </form>
</dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 20px;
    width: 560px;
    max-width: 100%;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 12px;
  }

  .wide {
    grid-column: 1 / -1;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  .host {
    grid-column: 1;
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  legend {
    padding: 0 6px;
    font-weight: 600;
  }

  p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .help {
    margin-top: -4px;
  }

  .warning-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .ok-text {
    color: var(--ok);
  }

  .buttons {
    display: flex;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }
</style>
