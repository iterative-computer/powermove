import { expect, test } from './helpers/app';

test('legacy color controls clean up transient connections and preserve detached drafts', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const create = () => PM.colorField(() => '#ff0000', () => {}, { local: true });
    const draft = create(), ephemeral = create(), container = document.createElement('div');
    container.append(ephemeral);
    document.body.append(container);
    container.remove();
    // Chromium still observes this removed subtree until its notification.
    // This draft enters it only after removal and has never been connected.
    container.append(draft);
    await new Promise(resolve => setTimeout(resolve, 0));
    const draftSurvived = draft.childElementCount > 0;
    const ephemeralReleased = ephemeral.childElementCount === 0;
    document.body.append(draft);
    await new Promise(resolve => setTimeout(resolve, 0));
    const attachedDraft = draft.childElementCount > 0;
    draft.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    return { draftSurvived, ephemeralReleased, attachedDraft, draftReleased: draft.childElementCount === 0 };
  });
  expect(result).toEqual({ draftSurvived: true, ephemeralReleased: true, attachedDraft: true, draftReleased: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
