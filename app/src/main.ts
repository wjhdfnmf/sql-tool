import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';
import { applyRememberedTheme } from './lib/settings.svelte';

// Before anything is drawn, so the window opens in the chosen theme.
applyRememberedTheme();

mount(App, { target: document.getElementById('app')! });
