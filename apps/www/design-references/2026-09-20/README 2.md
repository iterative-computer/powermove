# Desktop app references

Fresh captures of the running Powermove Electron app on 2026-09-20, using its existing e2e launch helper with an isolated disposable profile. These are the visual references for `src/lib/AppPreview.svelte`.

- `agent.png`: real Agent panel, connected test account, default starter prompts and composer.
- `mods.png`: real Mods panel, built-in extensions and empty Added by you group.
- `export.png`: real Export dialog with Code selected.

The website rebuilds these as decorative, accessible HTML illustrations using the shared app tokens. The sample export filename is changed from the test composition to `Your move-web.zip`. The export dialog keeps its captured dimensions and is cropped to its upper controls in the website frame. No controls submit work or modify app settings.

- `motion.png`: fresh curve-editor capture with Scale X and Scale Y selected in a disposable Your move composition. `motion-geometry.json` records keyframe positions and samples evaluated by the app. `static/motion-curves.svg` redraws the captured graph at its native coordinates for a crisp close-up.
- The Export illustration now preserves the captured 620px dialog width and crops into its Code/Video choices and integration summary. The feature frames use the same 16:9 sizing, so both close-ups scale as complete images on phones.

- `extensions.png`: actual Settings extension controls captured with two working sample extensions installed only in the disposable profile (Film grain effect and Notes panel). Film grain was switched off through Settings. The website remakes the native rows and switches at a narrower width, retaining native truncation and alignment. This replaces the curve-editor feature illustration.

- The website uses user-requested illustrative extension names, Motion tracking and Audio-reactive visuals, in the captured Settings row design. These placeholders are not presented as bundled extensions. The heading is omitted and the remaining panel is centered within the feature frame.
