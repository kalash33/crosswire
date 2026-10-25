// Playwright-only entry. Mounts the ByteEditor harness. This file is built into
// the bundle ONLY when E2E=1 is set (see vite.config.ts / playwright.config.ts),
// so it never reaches the committed production bundle.
import { mount } from 'svelte';
import Fixture from './Fixture.svelte';

const target = document.getElementById('fixture');
if (!target) throw new Error('fixture mount target missing');

mount(Fixture, { target });
