<script lang="ts">
  import { ChevronRight, CornerDownLeft, Plus } from '@lucide/svelte';

  let { kind, bare = false }: { kind: string; bare?: boolean } = $props();

  // UI details follow the app captures in design-references/2026-09-20.
</script>

<div class="app-preview" class:bare role="img" aria-label={kind === 'agent' ? 'Close-up of the Powermove prompt box with provider, model, and reasoning controls.' : kind === 'extensions' ? 'Example extensions in Powermove Settings, with Motion tracking switched off and Audio-reactive visuals switched on.' : 'Close-up of the Powermove Export dialog with the Video choice selected, showing format, resolution and frame rate.'}>
  {#if kind === 'agent'}
    <div class="agent-crop">
      <div class="agent" aria-hidden="true">
        <div class="composer">
          <div class="composer-field">
            <span>Describe a change…</span>
            <div class="composer-actions"><Plus size={14} /><CornerDownLeft size={13} /></div>
          </div>
          <div class="composer-options"><span>Codex</span><span>5.6 Sol</span><span>High</span></div>
        </div>
      </div>
    </div>
  {:else if kind === 'extensions'}
    <div class="preview-scene" aria-hidden="true">
      <div class="extension-settings">
        <div class="extensions-summary">Installed <span>2 extensions</span></div>
        <div class="extensions-list">
          <div class="extension-row">
            <div class="extension-copy"><div class="extension-name"><i></i><b>Motion tracking</b></div><span>Track subjects and attach layers to their motion.</span></div>
            <div class="extension-controls"><span class="extension-toggle"><i></i></span><ChevronRight size={12} /></div>
          </div>
          <div class="extension-row">
            <div class="extension-copy"><div class="extension-name"><i class="active"></i><b>Audio-reactive visuals</b></div><span>Drive effects and animation with sound.</span></div>
            <div class="extension-controls"><span class="extension-toggle on"><i></i></span><ChevronRight size={12} /></div>
          </div>
        </div>
      </div>
    </div>
  {:else}
    <div class="preview-scene">
      <div class="export-dialog" aria-hidden="true">
        <div class="export-heading">Export</div>
        <div class="export-body">
          <div class="export-tabs"><span class="selected">Video</span><span>Images</span><span>Project</span></div>
          <p class="export-hint">MP4, WebM or ProRes for delivery and editing.</p>
          <div class="export-section">Output</div>
          <div class="export-rows">
            <div class="export-row"><div>Format<p>Container and codec of the exported file.</p></div><span>MP4 · H.264</span></div>
            <div class="export-row"><div>Resolution<p>Rendered size relative to the composition.</p></div><span>Full · 1920×1080</span></div>
            <div class="export-row"><div>Frame rate<p>Frames written per second of exported video.</p></div><span>30 fps</span></div>
          </div>
        </div>
        <div class="export-footer"><span class="cancel">Cancel</span><span class="export-action">Export video</span></div>
      </div>
    </div>
  {/if}
</div>

<style>
  .app-preview{text-align:left;display:flex;align-items:center;justify-content:center;box-sizing:border-box;min-width:0;background:var(--bg-panel-2);font-family:var(--f-ui);font-size:13px;line-height:1.45;color:var(--tx);user-select:none}
  .app-preview{position:relative;width:min(calc(100% - 16px),1280px);aspect-ratio:16/9;padding:0;border-radius:var(--r-xl);box-shadow:var(--shadow-panel);overflow:hidden;isolation:isolate;container-type:inline-size}
  .app-preview.bare{width:100%;border-radius:0;box-shadow:none}
  .app-preview.bare::after{display:none}
  .app-preview::after{content:"";position:absolute;inset:0;border-radius:inherit;box-shadow:inset 0 0 0 1px rgb(var(--ink-rgb) / .08);pointer-events:none}
  .preview-scene{position:absolute;left:0;top:0;width:360px;height:202.5px;transform:scale(calc(100cqw / 360px));transform-origin:top left}
  .extension-settings{position:absolute;left:50%;top:50%;width:312px;transform:translate(-50%,-50%)}
  .extensions-summary{display:flex;gap:6px;padding:0 12px 8px;font-size:11px;line-height:16px;color:var(--tx-3)}
  .extensions-summary span{color:var(--tx-4)}
  .extensions-list{overflow:hidden;border-radius:16px;background:var(--bg-panel)}
  .extension-row{display:flex;align-items:center;gap:10px;height:56px;padding:0 10px 0 12px}
  .extension-copy{flex:1;min-width:0;display:flex;flex-direction:column;gap:1px;padding-right:4px}
  .extension-name{display:flex;align-items:center;gap:7px;line-height:18px}
  .extension-name b{font-size:13px;font-weight:500}
  .extension-name>i{width:6px;height:6px;flex:none;border-radius:50%;background:var(--tx-4)}
  .extension-name>i.active{background:var(--success)}
  .extension-copy>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:11px;line-height:16px;color:var(--tx-3)}
  .extension-controls{display:flex;align-items:center;gap:10px;flex:none;color:var(--tx-4)}
  .extension-toggle{position:relative;width:32px;height:18px;border-radius:999px;background:var(--ink-3)}
  .extension-toggle i{position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--bg-float);box-shadow:0 1px 2px rgb(0 0 0 / .18)}
  .extension-toggle.on{background:var(--tx)}
  .extension-toggle.on i{transform:translateX(14px);background:var(--bg-panel)}
  .agent-crop{position:absolute;inset:0;overflow:hidden}
  .agent{position:absolute;left:12.5%;top:50%;width:135%;font-size:4.875cqw;transform:translateY(-50%)}
  .composer{margin:0 .76923em;padding:.30769em .30769em 0;background:var(--ink-1);border-radius:.92308em;color:var(--tx-3)}
  .composer-field{padding:.76923em;background:var(--bg-panel);border-radius:.69231em}
  .composer-actions{display:flex;align-items:center;justify-content:space-between;margin-top:1.53846em;color:var(--tx-4)}
  .composer-actions :global(svg){width:1.07692em;height:1.07692em}
  .composer-actions :global(svg:last-child){width:1em;height:1em}
  .composer-options{display:flex;gap:2em;padding:.45455em .90909em;font-size:.84615em;color:var(--tx-2)}
  .composer-options span:last-child{color:var(--tx-3)}
  .export-dialog{position:absolute;left:24px;top:20px;display:flex;flex-direction:column;width:620px;min-height:624px;border-radius:16px;overflow:hidden;background:var(--bg-float)}
  .export-heading{padding:16px 18px 0;font-size:15px;font-weight:600;letter-spacing:-.01em}
  .export-body{flex:1;padding:10px 18px 18px}
  .export-tabs{display:flex;gap:2px;padding:3px;background:var(--bg-panel);border-radius:8px;font-size:13px;color:var(--tx-2)}
  .export-tabs span{flex:1;text-align:center;padding:4px 2px;border-radius:5px}
  .export-tabs .selected{background:var(--ink-1);color:var(--tx)}
  .export-hint{margin:10px 14px 22px;font-size:12px;color:var(--tx-3)}
  .export-section{margin:0 14px 8px;font-size:12px;color:var(--tx-3)}
  .export-rows{padding:4px 14px;background:var(--bg-panel);border-radius:18px;font-size:13px}
  .export-row{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:10px 0}
  .export-row p{margin-top:2px;color:var(--tx-3);font-size:12px}
  .export-row span{flex:none;padding:5px 10px;border-radius:8px;background:var(--ink-1);font-size:12px}
  .export-footer{display:flex;justify-content:flex-end;gap:8px;padding:12px 18px;border-top:1px solid var(--line);font-size:12px}
  .export-footer span{padding:7px 12px;border-radius:8px}
  .cancel{background:var(--ink-1)}.export-action{background:var(--accent);color:var(--on-accent)}
</style>
