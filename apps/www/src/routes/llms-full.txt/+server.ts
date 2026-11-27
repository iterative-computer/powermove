import { llmsFullTxt } from '$lib/llms';
import { latestDownload } from '$lib/server/release';
import type { RequestHandler } from './$types';

export const prerender = true;

export const GET: RequestHandler = async ({ fetch }) => {
  const download = await latestDownload(fetch);
  return new Response(llmsFullTxt(download.href), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
