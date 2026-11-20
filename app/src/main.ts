import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';
import { watchForFaults } from './lib/faults.svelte';
import { applyRememberedTheme } from './lib/settings.svelte';

// Before anything is drawn, so the window opens in the chosen theme.
applyRememberedTheme();
// Before the interface exists, so that a failure while it is being drawn is caught too.
watchForFaults();

mount(App, { target: document.getElementById('app')! });
