<script lang="ts">
  import { onMount } from 'svelte';

  interface Props {
    title: string;
    user: string;
    error: string | null;
    busy: boolean;
    oncancel: () => void;
    onsubmit: (password: string, remember: boolean) => void;
  }

  let { title, user, error, busy, oncancel, onsubmit }: Props = $props();

  let dialog: HTMLDialogElement;
  let password = $state('');
  let remember = $state(false);

  onMount(() => dialog.showModal());
</script>

<dialog bind:this={dialog} oncancel={oncancel} aria-labelledby="password-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onsubmit(password, remember);
    }}
  >
    <h2 id="password-title">Password for {title}</h2>
    <label>
      <span>Password for user <strong>{user}</strong></span>
      <!-- svelte-ignore a11y_autofocus -->
      <input type="password" bind:value={password} autocomplete="off" autofocus />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={remember} />
      Save in the system keychain
    </label>
    {#if error}<p class="error-text" role="alert">{error}</p>{/if}
    <div class="buttons">
      <button type="button" onclick={oncancel}>Cancel</button>
      <button type="submit" class="primary" disabled={busy}>{busy ? 'Connecting…' : 'Connect'}</button>
    </div>
  </form>
</dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    width: 380px;
    max-width: 100%;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }

  p {
    margin: 0;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
