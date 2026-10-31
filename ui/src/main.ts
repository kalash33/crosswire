import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { applyStoredTheme } from './lib/theme';

// Before the first mount so the app paints in the chosen theme.
applyStoredTheme();

const target = document.getElementById('app');
if (!target) {
  throw new Error('Mount target #app not found');
}

const app = mount(App, { target });

export default app;
