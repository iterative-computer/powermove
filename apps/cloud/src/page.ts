/* The few HTML pages the Worker serves itself (verification, desktop sign-in hand-off).
   Colors, radii and type mirror @powermove/tokens so they read as part of the website.
   Everything is inline: callers' CSPs allow no fonts, images or external styles. */

const mark = '<svg class="mark" viewBox="0 0 59.41 164.75" aria-hidden="true"><defs><linearGradient id="pm-trail" x1="1" y1="0" x2="0" y2="0"><stop offset="0" stop-color="#F0580A" stop-opacity="0.71"/><stop offset="1" stop-color="#F0580A" stop-opacity="0"/></linearGradient><linearGradient id="pm-fade" x1="0" y1="0" x2="0" y2="1"><stop offset="0.55" stop-color="#fff"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient><mask id="pm-mask" maskContentUnits="objectBoundingBox"><rect width="1" height="1" fill="url(#pm-fade)"/></mask></defs><rect x="5.26" y="56.41" width="26.39" height="108.34" fill="url(#pm-trail)" mask="url(#pm-mask)"/><rect x="31.65" y="56.41" width="5.0" height="108.34" fill="#F0580A"/><path d="M9.42 0.06L50.58 0.00C54.01-0.00 57.14 1.99 58.60 5.11C60.04 8.23 59.54 11.91 57.31 14.53L47.91 25.61C46.55 27.14 45.83 28.88 45.65 30.87L43.33 44.83C42.76 48.31 40.16 51.13 36.73 51.96C33.30 52.81 29.69 51.52 27.57 48.69L1.99 14.62C-2.76 7.80 1.49 0.78 9.42 0.06Z" fill="#F0580A"/></svg>';

export const icons = {
  spinner: '<svg class="icon spin" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="2" opacity=".2"/><path d="M21 12a9 9 0 0 0-9-9" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  check: '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 12.5 4 4 8-9" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  alert: '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 6v8M12 18v.01" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"/></svg>'
};

const style = `
:root{color-scheme:dark;--ink:245 245 244;--bg:#0C0A09;--panel:#1C1917;--line:rgb(var(--ink) / .08);--tx:#F5F5F4;--tx-2:rgb(var(--ink) / .62);--tx-3:rgb(var(--ink) / .44);--accent:#FF6B1A;--accent-hover:#FF7B33;--accent-dim:rgba(255,107,26,.15);--danger:#F36357;--danger-dim:rgba(243,99,87,.15);--success:#3FCF8E;--success-dim:rgba(63,207,142,.15);--on-accent:#fff}
@media (prefers-color-scheme:light){:root{color-scheme:light;--ink:20 22 28;--bg:#EEEEF1;--panel:#FDFDFE;--tx:#1B1D23;--tx-2:rgb(var(--ink) / .66);--tx-3:rgb(var(--ink) / .48);--accent:#F0580A;--accent-hover:#FF6A24;--accent-dim:rgba(240,88,10,.12);--danger:#D93025;--danger-dim:rgba(217,48,37,.1);--success:#148F5E;--success-dim:rgba(20,143,94,.1)}}
*{box-sizing:border-box}
html,body{height:100%}
body{margin:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:28px;padding:48px 16px;background:var(--bg);color:var(--tx);font:14px/1.5 Geist,-apple-system,BlinkMacSystemFont,system-ui,sans-serif;-webkit-font-smoothing:antialiased;text-align:center}
.brand{display:flex;align-items:center;gap:9px;font-size:15px;font-weight:500;letter-spacing:-.02em}
.mark{width:9px;height:25px}
.card{width:100%;max-width:400px;padding:36px 32px 32px;border-radius:12px;background:var(--panel);box-shadow:0 0 0 1px var(--line),0 24px 48px -24px rgb(0 0 0 / .45);display:flex;flex-direction:column;align-items:center}
.badge{display:grid;place-items:center;width:44px;height:44px;margin-bottom:20px;border-radius:50%;background:var(--accent-dim);color:var(--accent)}
.badge[data-tone=success]{background:var(--success-dim);color:var(--success)}
.badge[data-tone=danger]{background:var(--danger-dim);color:var(--danger)}
.icon{width:22px;height:22px}
.spin{animation:spin .8s linear infinite}
@keyframes spin{to{transform:rotate(360deg)}}
@media (prefers-reduced-motion:reduce){.spin{animation-duration:2.4s}}
h1{margin:0;font-size:20px;font-weight:500;letter-spacing:-.02em;line-height:1.25;text-wrap:balance}
p{margin:8px 0 0;color:var(--tx-2);text-wrap:pretty;max-width:32ch}
.button{display:inline-flex;align-items:center;justify-content:center;min-height:40px;margin-top:24px;padding:0 18px;border-radius:6px;background:var(--accent);color:var(--on-accent);font-weight:500;text-decoration:none;transition:background 100ms}
.button:hover{background:var(--accent-hover)}
.button:focus-visible{outline:0;box-shadow:0 0 0 2px var(--panel),0 0 0 4px color-mix(in srgb,var(--accent) 65%,transparent)}
.foot{margin:0;font-size:12px;color:var(--tx-3)}
[hidden]{display:none!important}
`;

/** A centered, branded card. `body` is trusted markup; escape anything user-derived before passing it. */
export function page({ title, nonce, body, foot = '', head = '' }: { title: string; nonce: string; body: string; foot?: string; head?: string }): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">${head}<title>${title} · Powermove</title><style nonce="${nonce}">${style}</style></head><body><div class="brand">${mark}Powermove</div><main class="card">${body}</main>${foot ? `<p class="foot">${foot}</p>` : ''}</body></html>`;
}
