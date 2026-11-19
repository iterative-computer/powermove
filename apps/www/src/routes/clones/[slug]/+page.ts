import { error } from '@sveltejs/kit';
import { CLONES, cloneBySlug } from '$lib/clones';
import type { EntryGenerator, PageLoad } from './$types';

export const entries: EntryGenerator = () => CLONES.map((clone) => ({ slug: clone.slug }));

export const load: PageLoad = ({ params }) => {
  const clone = cloneBySlug(params.slug);
  if (!clone) error(404, 'No clone with that name');
  return { clone };
};
