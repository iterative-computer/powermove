/* The static, read-only share page. Everything is inline and escaped; the CSP
   allows only this nonce'd stylesheet and same-origin images (the poster). */
import type { ShareSnapshot } from './routes/shares';

const esc = (value: unknown) => String(value ?? '').replace(/[&<>"']/g, ch => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[ch]!);
const mark = '<svg class="mark" viewBox="0 0 141 116" aria-hidden="true"><path fill="currentColor" d="M33.3885 106.544L5.19736 115.218C1.26974 116.426 -1.70853 111.636 1.11257 108.648L28.2054 79.9531C42.9061 64.3828 53.2117 45.1902 58.0699 24.3349L63.0183 3.09249C63.9876 -1.06813 69.929 -1.01597 70.8251 3.16102L74.1448 18.6367C79.385 43.0646 92.058 65.2717 110.424 82.2092L138.829 108.405C141.918 111.254 139.086 116.33 135.039 115.198L101.016 105.677C78.8565 99.4757 55.3816 99.7767 33.3885 106.544Z"/></svg>';

export function timecode(seconds: number): string {
  const s = Math.max(0, seconds), m = Math.floor(s / 60), rest = s - m * 60;
  return `${m}:${rest.toFixed(rest % 1 ? 1 : 0).padStart(rest % 1 ? 4 : 2, '0')}`;
}

const style = `
:root{color-scheme:dark;--ink:245 245 244;--bg:#0C0A09;--panel:#1C1917;--field:#252220;--line:rgb(var(--ink) / .08);--tx:#F5F5F4;--tx-2:rgb(var(--ink) / .62);--tx-3:rgb(var(--ink) / .44);--accent:#FF6B1A;--accent-dim:rgba(255,107,26,.15)}
@media (prefers-color-scheme:light){:root{color-scheme:light;--ink:20 22 28;--bg:#EEEEF1;--panel:#FDFDFE;--field:#F3F3F6;--tx:#1B1D23;--tx-2:rgb(var(--ink) / .66);--tx-3:rgb(var(--ink) / .48);--accent:#F0580A;--accent-dim:rgba(240,88,10,.12)}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--tx);font:14px/1.5 Geist,-apple-system,BlinkMacSystemFont,system-ui,sans-serif;-webkit-font-smoothing:antialiased}
.wrap{max-width:880px;margin:0 auto;padding:28px 16px 64px}
header{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:28px}
.brand{display:flex;align-items:center;gap:9px;font-size:15px;font-weight:500;letter-spacing:-.02em;color:var(--tx);text-decoration:none}
.mark{width:22px;height:18px}
.ro{font-size:12px;color:var(--tx-3);display:flex;align-items:center;gap:6px}
.ro i{width:6px;height:6px;border-radius:50%;background:var(--tx-3)}
.card{background:var(--panel);border-radius:12px;box-shadow:0 0 0 1px var(--line),0 24px 48px -24px rgb(0 0 0 / .45);overflow:hidden}
.poster{display:block;width:100%;height:auto;background:#000;border-bottom:1px solid var(--line)}
.blank{aspect-ratio:16/9;display:grid;place-items:center;background:var(--field);color:var(--tx-3);border-bottom:1px solid var(--line)}
.blank .mark{width:48px;height:40px;opacity:.5}
.body{padding:24px 24px 28px}
.kind{font-size:11px;font-weight:500;letter-spacing:.06em;text-transform:uppercase;color:var(--accent)}
h1{margin:6px 0 0;font-size:24px;font-weight:500;letter-spacing:-.02em;line-height:1.2;text-wrap:balance}
.tagline{margin:6px 0 0;color:var(--tx-2)}
.facts{display:flex;flex-wrap:wrap;gap:6px;margin:16px 0 0;padding:0;list-style:none}
.facts li{padding:3px 9px;border-radius:6px;background:var(--field);color:var(--tx-2);font-size:12px;font-variant-numeric:tabular-nums}
.desc{margin:18px 0 0;color:var(--tx-2);white-space:pre-wrap;max-width:68ch}
h2{margin:28px 0 8px;font-size:12px;font-weight:500;color:var(--tx-3);letter-spacing:.02em}
table{width:100%;border-collapse:collapse;font-size:13px}
td{padding:7px 0;border-top:1px solid var(--line);vertical-align:top}
td.t{color:var(--tx-3);width:120px}
td.n{text-align:right;color:var(--tx-3);font-variant-numeric:tabular-nums;white-space:nowrap}
pre{margin:0;padding:14px 16px;border-radius:8px;background:var(--field);white-space:pre-wrap;word-break:break-word;font:12.5px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;color:var(--tx-2)}
footer{margin-top:20px;text-align:center;font-size:12px;color:var(--tx-3)}
footer a{color:var(--tx-2)}
.missing{text-align:center;padding:72px 24px}
.missing p{color:var(--tx-2)}
`;

export function renderSharePage(share: { id: string; snapshot: ShareSnapshot; hasPoster: boolean; createdAt: Date } | null, nonce: string): string {
  const shell = (title: string, inner: string) => `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>${esc(title)} · Powermove</title><style nonce="${nonce}">${style}</style></head><body><div class="wrap"><header><a class="brand" href="https://trypowermove.com">${mark}Powermove</a><span class="ro"><i></i>Read-only view</span></header>${inner}<footer>Shared from Powermove. <a href="https://trypowermove.com">Make your own</a></footer></div></body></html>`;
  if (!share) return shell('Link unavailable', `<main class="card missing"><h1>This link isn't available</h1><p>It may have been removed by the person who shared it.</p></main>`);
  const s = share.snapshot;
  const visual = share.hasPoster ? `<img class="poster" src="/s/${esc(share.id)}/poster" alt="">` : `<div class="blank">${mark}</div>`;
  let facts: string[] = [], detail = '';
  if (s.kind === 'project') {
    facts = [`${s.width} × ${s.height}`, `${+s.fps.toFixed(3)} fps`, timecode(s.durationSeconds)];
    if (s.compositions.length > 1) facts.push(`${s.compositions.length} compositions`);
    if (s.layers.length) {
      facts.push(`${s.layers.length} layers`);
      detail = `<h2>Layers</h2><table>${s.layers.map(l => `<tr><td>${esc(l.name)}</td><td class="t">${esc(l.type)}</td><td class="n">${l.inSeconds !== undefined && l.outSeconds !== undefined ? `${timecode(l.inSeconds)}–${timecode(l.outSeconds)}` : ''}</td></tr>`).join('')}</table>`;
    }
  } else {
    if (s.handle) facts.push(`@${s.handle}`);
    if (s.version) facts.push(`v${s.version}`);
    facts.push(...s.permissions);
    if (s.contributes.length) detail += `<h2>Adds</h2><table>${s.contributes.map(x => `<tr><td>${esc(x.name)}</td><td class="t">${esc(x.kind)}</td></tr>`).join('')}</table>`;
    if (s.readme) detail += `<h2>Readme</h2><pre>${esc(s.readme)}</pre>`;
  }
  const tagline = s.kind === 'extension' && s.tagline ? `<p class="tagline">${esc(s.tagline)}</p>` : '';
  return shell(s.title, `<main class="card">${visual}<div class="body"><div class="kind">${s.kind === 'project' ? 'Project' : 'Extension'}</div><h1>${esc(s.title)}</h1>${tagline}<ul class="facts">${facts.map(f => `<li>${esc(f)}</li>`).join('')}</ul>${s.description ? `<p class="desc">${esc(s.description)}</p>` : ''}${detail}</div></main>`);
}
