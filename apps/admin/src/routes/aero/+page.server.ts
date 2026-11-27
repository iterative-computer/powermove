import { fail } from '@sveltejs/kit';
import { Admin } from '@powermove/registry/wire';
import { cloud, problem } from '$lib/server/cloud';
import { requireAdmin } from '$lib/server/session';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async (event) => {
  requireAdmin(event.locals);
  return { overview: await cloud(event, '/v1/admin/aero/overview', { schema: Admin.AeroOverview.Res }) };
};

export const actions: Actions = {
  unblock: async (event) => {
    requireAdmin(event.locals);
    const userId = String((await event.request.formData()).get('userId') ?? '');
    try {
      await cloud(event, `/v1/admin/aero/blocks/${encodeURIComponent(userId)}`, { method: 'DELETE', schema: Admin.AeroUnblock.Res });
    } catch (e) {
      const { status, message } = problem(e);
      return fail(status, { message });
    }
    return { message: null };
  },
};
