<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { backend } from '../lib/backend';
  import {
    asUiError,
    explainConnectError,
    formatFingerprint,
    serverLabel,
    type CertificateInfo,
    type Profile,
    type TlsSetting,
  } from '../lib/types';

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
  /** `fix` names a one-click remedy to offer alongside the message. */
  let testResult = $state<{ ok: boolean; text: string; fix?: 'unverified' | 'cleartext' } | null>(null);
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
      help: 'Checks the certificate against the CA file but not the host name. For servers whose certificate does not name them, such as MySQL auto-generated certificates.',
    },
    {
      value: 'pinned',
      label: 'Encrypted, pinned certificate',
      help: "Trusts one specific certificate that you approve below. No CA file is needed, and a server that presents any other certificate is refused. If the server's certificate is replaced, you approve the new one.",
    },
    {
      value: 'unverified',
      label: 'Encrypted, not verified',
      help: 'Stops eavesdropping, but not a server that impersonates yours: one that does can read your password and everything you send. This is what most MySQL tools do by default. Pinning the certificate closes that gap and needs no CA file.',
    },
    {
      value: 'disabled',
      label: 'Not encrypted',
      help: 'Everything, including the login exchange, crosses the network unprotected.',
    },
  ];

  const colors = [null, 'red', 'orange', 'green', 'blue', 'purple'];

  const tlsHelp = $derived(tlsOptions.find((o) => o.value === draft.tls)?.help ?? '');
  const usesCa = $derived(draft.tls === 'verify-identity' || draft.tls === 'verify-ca');
  const verifies = $derived(usesCa || draft.tls === 'pinned');
  const needsRiskAck = $derived(draft.cleartextAuth && !verifies);

  // Certificate pinning: fetch what the server presents, then trust it or not.
  let fetched = $state<CertificateInfo | null>(null);
  let fetching = $state(false);
  let fetchError = $state<string | null>(null);

  async function fetchCertificate() {
    fetching = true;
    fetchError = null;
    fetched = null;
    try {
      fetched = await backend.certificateProbe(draft.host.trim(), draft.port);
    } catch (e) {
      fetchError = asUiError(e).message;
    } finally {
      fetching = false;
    }
  }

  function trustFetched() {
    if (!fetched) return;
    draft.pinnedSha256 = fetched.sha256;
    fetched = null;
    testResult = null;
  }
  const problem = $derived.by(() => {
    if (draft.host.trim() === '') return 'Enter the server host name or address.';
    if (draft.user.trim() === '') return 'Enter the user name.';
    if (!Number.isInteger(draft.port) || draft.port < 1 || draft.port > 65535) return 'Port must be between 1 and 65535.';
    if (needsRiskAck && !draft.cleartextRiskAccepted) return 'Accept the cleartext risk below, or choose a verified encryption mode.';
    if (draft.tls === 'pinned' && !draft.pinnedSha256) return "Fetch the server's certificate and trust it, or choose another encryption mode.";
    return null;
  });

  onMount(() => dialog.showModal());

  function current(): Profile {
    return {
      ...draft,
      host: draft.host.trim(),
      user: draft.user.trim(),
      database: database.trim() || null,
      caFile: usesCa && caFile.trim() ? caFile.trim() : null,
      pinnedSha256: draft.tls === 'pinned' ? draft.pinnedSha256 : null,
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
        text: error.kind === 'password-required' ? 'Enter the password to test the connection.' : explainConnectError(error),
        fix: error.kind === 'tls' && usesCa ? 'unverified' : error.kind === 'cleartext-not-enabled' ? 'cleartext' : undefined,
      };
    } finally {
      testing = false;
    }
  }

  function applyFix(fix: 'unverified' | 'cleartext' | 'pin') {
    if (fix === 'pin') {
      draft.tls = 'pinned';
      testResult = null;
      void fetchCertificate();
      return;
    }
    if (fix === 'unverified') draft.tls = 'unverified';
    else draft.cleartextAuth = true;
    testResult = {
      ok: true,
      text:
        fix === 'unverified'
          ? 'Encryption is now "Encrypted, not verified". Test the connection again.'
          : 'Cleartext authentication is now allowed. Check the security section, then test again.',
    };
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

      {#if draft.tls === 'pinned'}
        <div class="pin">
          {#if draft.pinnedSha256}
            <div>
              <span class="muted">Trusted certificate (SHA-256)</span>
              <code>{formatFingerprint(draft.pinnedSha256)}</code>
            </div>
          {:else}
            <p class="muted">No certificate is trusted for this connection yet.</p>
          {/if}
          <div>
            <button type="button" disabled={fetching || draft.host.trim() === ''} onclick={fetchCertificate}>
              {fetching ? 'Fetching…' : draft.pinnedSha256 ? "Fetch the server's certificate again" : "Fetch the server's certificate"}
            </button>
            <span class="muted">No user name or password is sent.</span>
          </div>
          {#if fetchError}<p class="error-text" role="alert">{fetchError}</p>{/if}
          {#if fetched}
            <div class="warning-box" role="group" aria-label="Certificate presented by the server">
              <dl>
                <dt>Issued to</dt>
                <dd>{fetched.subject}</dd>
                <dt>Issued by</dt>
                <dd>{fetched.issuer}{fetched.selfSigned ? ' (self-signed)' : ''}</dd>
                <dt>Expires</dt>
                <dd>{fetched.notAfter}</dd>
                <dt>SHA-256</dt>
                <dd><code>{formatFingerprint(fetched.sha256)}</code></dd>
              </dl>
              {#if fetched.sha256 === draft.pinnedSha256}
                <p>This is the certificate already trusted.</p>
              {:else}
                <p>
                  Trust it only if you are confident this is your server, for example by comparing the fingerprint
                  with one from your database administrator.
                </p>
                <div><button type="button" onclick={trustFetched}>Trust This Certificate</button></div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}

      {#if usesCa}
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

    <fieldset>
      <legend>Safety</legend>
      <label class="check">
        <input type="checkbox" bind:checked={draft.readOnly} />
        Read-only: refuse anything that changes data or structure
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={draft.production} />
        Production: ask before structure changes, and before UPDATE or DELETE without WHERE
      </label>
    </fieldset>

    {#if testResult}
      <div class="test-result" role="status">
        <p class={testResult.ok ? 'ok-text' : 'error-text'}>{testResult.text}</p>
        {#if testResult.fix === 'unverified'}
          <button type="button" onclick={() => applyFix('pin')}>Pin This Server's Certificate</button>
          <button type="button" onclick={() => applyFix('unverified')}>Switch to "Encrypted, not verified"</button>
        {:else if testResult.fix === 'cleartext'}
          <button type="button" onclick={() => applyFix('cleartext')}>Allow cleartext authentication</button>
        {/if}
      </div>
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

  .test-result {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }

  .pin {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .pin code {
    display: block;
    font-family: var(--mono);
    font-size: 11px;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .pin dl {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 10px;
    margin: 0 0 8px;
  }

  .pin dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .pin .warning-box p {
    margin: 0 0 8px;
  }

  .buttons {
    display: flex;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }
</style>
