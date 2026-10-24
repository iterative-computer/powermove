import { VISUAL_REPLY_CSP, VISUAL_REPLY_MAX_BYTES, visualReplyInput } from '../../../../shared/visual-reply';

/** Reject ambient resources. Local images must already be bounded embedded raster bytes.
 * This API never accepts paths, so publishing cannot become a filesystem read oracle. */
export function prepareVisualReply(value: unknown): ReturnType<typeof visualReplyInput> {
  const input = visualReplyInput(value);
  // Template contents are inert even for images/iframes. DOMParser's detached
  // HTML documents may fetch resources before validation has rejected them.
  const template = document.createElement('template');
  template.innerHTML = input.html;
  const content = template.content;
  if (content.querySelector('base, meta[http-equiv], iframe, frame, frameset, object, embed, link, form, template')) {
    throw new Error('Visual replies cannot embed pages, forms, metadata redirects or external resources. Use inline CSS, SVG and scripts.');
  }
  for (const node of content.querySelectorAll('*')) for (const attr of Array.from(node.attributes)) {
    if (['src','srcset','href','xlink:href','action','formaction','poster','background','ping'].includes(attr.name.toLowerCase())) {
      const v = attr.value;
      if ((attr.name === 'href' || attr.name === 'xlink:href') && v.startsWith('#')) continue;
      if (attr.name === 'src' && node.tagName === 'IMG' && /^data:image\/(png|jpeg|webp|gif);base64,[A-Za-z0-9+/]+=*$/.test(v) && v.length <= 256 * 1024) {
        const [mime, bytes] = v.split(',');
        const head = atob(bytes!.slice(0, 24));
        const raster = mime === 'data:image/png;base64' ? head.startsWith('\x89PNG\r\n\x1a\n')
          : mime === 'data:image/jpeg;base64' ? head.startsWith('\xff\xd8\xff')
          : mime === 'data:image/gif;base64' ? /^GIF8[79]a/.test(head)
          : head.startsWith('RIFF') && head.slice(8, 12) === 'WEBP';
        if (!raster) throw new Error('An embedded image does not match its raster format.');
        continue;
      }
      throw new Error('Visual reply resources must be embedded raster images (up to 192 KiB each) or fragment links. File paths and remote links are unavailable.');
    }
  }
  // Persist the normalized source, never trust a previously prepared archive at render time.
  const html = '<!doctype html><html><head></head><body>' + template.innerHTML + '</body></html>';
  if (new TextEncoder().encode(html).length > VISUAL_REPLY_MAX_BYTES) throw new Error('Normalized visual reply exceeds 384 KiB.');
  return { ...input, html };
}
export function visualDocument(html: string, token: string): string {
  // Policy is enforced by both the dedicated document response and this first meta.
  const boot = `(() => {
    const token = ${JSON.stringify(token)};
    const send = (type, rest={}) => parent.postMessage({channel:'powermove-visual',token,type,...rest}, '*');
    addEventListener('error', () => send('error',{message:'The visualization encountered a script error. Replay it or ask the agent to fix it.'}));
    addEventListener('unhandledrejection', () => send('error',{message:'The visualization could not finish an interaction.'}));
    addEventListener('securitypolicyviolation', () => send('error',{message:'A resource was blocked. Use self-contained HTML with embedded images.'}));
    addEventListener('click', e => { if(e.target.closest?.('a')) { e.preventDefault(); e.stopImmediatePropagation(); send('error',{message:'Links stay inside this visualization. External navigation is unavailable.'}); } }, true);
    addEventListener('submit', e => e.preventDefault(), true);
    addEventListener('message', e => {
      if(e.source !== parent || e.data?.channel !== 'powermove-visual-theme' || e.data.token !== token) return;
      const colors = e.data.colors;
      if(!colors || typeof colors !== 'object') return;
      for(const key of ['--background','--foreground','--muted','--accent']) {
        if(typeof colors[key] === 'string' && colors[key].length <= 100 && CSS.supports('color', colors[key])) document.documentElement.style.setProperty(key,colors[key]);
      }
      document.documentElement.style.colorScheme = e.data.dark ? 'dark' : 'light';
    });
    addEventListener('DOMContentLoaded', () => {
      send('ready');
      let queued = false;
      new ResizeObserver(() => { if(queued) return; queued=true; requestAnimationFrame(() => { queued=false; send('size',{height:Math.min(100000,Math.max(document.body.scrollHeight,document.body.getBoundingClientRect().height))}); }); }).observe(document.body);
    }, {once:true});
  })();`;
  const prefix = `<meta http-equiv="Content-Security-Policy" content="${VISUAL_REPLY_CSP}"><meta name="referrer" content="no-referrer"><style>:root{--background:#202022;--foreground:#eee;--muted:#999;--accent:#a7a0ff;color-scheme:dark}html{background:var(--background);color:var(--foreground);font:13px system-ui}body{margin:0;padding:16px;box-sizing:border-box}*{box-sizing:border-box}button,input,select{font:inherit}a{color:var(--accent)}</style><script>${boot}<` + '/script>';
  return html.replace(/<head[^>]*>/i, m => m + prefix);
}
