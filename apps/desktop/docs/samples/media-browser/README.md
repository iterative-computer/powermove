# Media Browser sample

This teaching extension registers a quiet left-docked panel, loads a replaceable
image catalogue, imports selected files into Powermove, and starts typed timeline
drags. Copy the whole folder into an extension directory to try it.

## Follow the data

1. `mockMediaSource` returns `{ id, name, url }` records. Replace only that
   function when connecting a real catalogue.
2. The panel fetches a URL as a `Blob`, wraps it in a named `File`, and calls
   `api.assets.import`. This API returns the durable asset id needed by drag/drop.
3. `api.dnd.startAssetDrag` writes the canonical asset MIME payload. The sample
   also sets `api.dnd.mediaDrag` during the gesture so the timeline can describe
   the drop preview while browsers protect `DataTransfer` contents.
4. Dropping creates one undoable project edit. `api.project.undo()` removes that
   layer; the durable imported asset intentionally remains available.

The default catalogue uses `data:` URLs, so it needs no server and avoids
`connect-src` restrictions. If the app's Content Security Policy blocks a remote
catalogue, proxy it through an allowed origin or return data URLs from the source.

Only `powermove`, Svelte, and files inside this folder are imported. No renderer
internals or legacy registry are involved.
