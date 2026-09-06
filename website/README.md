# Powermove website

A placeholder landing page for powermove.motioner.app, built with Geist, React, Vinext, and a native WebGL fragment shader.

## Develop

Run `npm ci`, then `npm run dev`. Run `npm run build` and `npm exec tsc -- --noEmit` before publishing.

## Preview interaction

The prompt box is a guided demo, not a connected AI service. Three prepared screenshots from an isolated Powermove editor session show the original composition, revised title, and violet atmosphere. Suggested prompts and revision buttons select the same three states. Unrecognized prompts explain the demo boundary. Nothing is submitted to a server or retained.

Background motion can be paused and respects reduced-motion preferences. WebGL has a CSS fallback. Geist is loaded through the framework's Google font integration.

## Hosting

The Sites project is recorded in `.openai/hosting.json`. Publication requires a source commit and Sites version. Custom-domain DNS must be configured with the records returned by Sites after the first publication.

Only this directory belongs to the website change. The parent Powermove editor has unrelated work in progress; do not stage or commit it with the website.
