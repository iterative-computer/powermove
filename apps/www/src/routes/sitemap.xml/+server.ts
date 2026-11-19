import { PAGES, absolute } from '$lib/site';

export const prerender = true;

export function GET() {
  const urls = PAGES.map((page) => `  <url><loc>${absolute(page.path === '/' ? '/' : page.path)}</loc></url>`).join('\n');
  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`;
  return new Response(body, { headers: { 'Content-Type': 'application/xml; charset=utf-8' } });
}
