import { mount } from 'svelte';

import App from './App.svelte';

const target = document.getElementById('app');

if (target === null) {
  throw new Error('Renderer mount target was not found');
}

mount(App, { target });
