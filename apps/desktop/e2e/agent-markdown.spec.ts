import { test, expect } from './helpers/app';

for (const theme of ['dark', 'light']) {
  test(`Markdown appears throughout the agent panel in ${theme} mode`, async ({ session }, testInfo) => {
    await session.openEditor(); await session.openAgent();
    const { page } = session;
    await page.evaluate((theme) => {
      const PM = (window as any).PM;
      PM.SpatialAssistant.open();
      PM.theme.apply(theme);
      PM.AgentUI.update({ flush: true });
      PM.AgentUI.update = () => {};
      const text = '## Review\n\n**Bold** and *italic* with `code` and ~~removed~~.\n\n- First item\n- Second item\n\n1. Ordered\n\n> Quoted text\n\n```ts\nconst value = 1;\n```\n\n| Name | Value |\n| --- | ---: |\n| Clip | **Ready** |\n\n---\n\n[Guide](https://example.com/guide)';
      const question = { kind: 'question', id: 'question', transport: 'reply', blocking: true, status: 'open', questions: [{ id: 'style', header: '**Style**', question: 'Choose **bold** or *italic*?', allowOther: true, secret: false, options: [{ label: '**Bold**', description: 'Keep *emphasis*' }] }] };
      Object.assign(PM.AgentUI.state, {
        phase: 'running', legacyPhase: 'running', activity: '', panelRun: null,
        conversation: [
          { role: 'user', text: '**Review** [Attachment: clip.mp4] and *trim* it.', attachments: [{ name: 'clip.mp4', path: '/tmp/clip.mp4' }] },
          { role: 'trace', steps: [{ kind: 'text', id: 'saved', text }, { kind: 'thought', id: 'saved-thought', label: '**Saved** reasoning', live: false }, { ...question, id: 'answered', status: 'answered', answers: { style: '**Bold**' } }] },
          { role: 'assistant', text },
          { role: 'assistant', text: '**Attention**: review `timing`.', error: true, notice: 'alert' },
          { role: 'assistant', text: '**Nothing** changed.', error: true, notice: 'plain' }
        ],
        trace: [{ kind: 'thought', id: 'live-thought', label: '**Live** reasoning', live: false }, question, { kind: 'text', id: 'live', text }]
      });
    }, theme);
    await page.setViewportSize({ width: 1440, height: 1100 });
    const panel = page.locator('#panel-agent');
    await panel.evaluate(el => (el.closest('#agent-popover') as HTMLElement).style.height = '950px');
    for (const selector of ['.agent-trace.is-archived .agent-trace-prose:not(.agent-thought-prose)', '.agent-msg.assistant .agent-reply', '.agent-trace.is-live .agent-trace-prose:not(.agent-thought-prose)']) {
      const prose = panel.locator(selector).first();
      await expect(prose.locator('.agent-md-h')).toHaveText('Review');
      await expect(prose.locator('.is-bold').first()).toHaveText('Bold');
      await expect(prose.locator('.is-italic')).toHaveText('italic');
      await expect(prose.locator('.is-struck')).toHaveText('removed');
      await expect(prose.locator('.agent-md-li')).toHaveCount(3);
      await expect(prose.locator('blockquote')).toContainText('Quoted text');
      await expect(prose.locator('pre code')).toHaveText('const value = 1;');
      await expect(prose.locator('table tbody td').last()).toHaveText('Ready');
      await expect(prose.locator('hr')).toHaveCount(1);
      await expect(prose.locator('a')).toHaveAttribute('href', 'https://example.com/guide');
    }
    await expect(panel.locator('.agent-bubble .is-bold')).toHaveText('Review');
    await expect(panel.locator('.agent-bubble .is-italic')).toHaveText('trim');
    await expect(panel.locator('.agent-question-option .is-italic')).toHaveText('emphasis');
    await expect(panel.locator('.agent-question-answer .is-bold')).toHaveText('Bold');
    await expect(panel.locator('.alert-text .is-bold')).toHaveText('Attention');
    await expect(panel.locator('.agent-thought-prose .is-bold')).toHaveText(['Saved', 'Live']);
    for (const width of [240, 320, 480]) {
      await panel.evaluate((el, width) => { (el.closest('#agent-popover') as HTMLElement).style.width = `${width}px`; }, width);
      const styles = await panel.locator('.agent-trace.is-live .agent-md-h').evaluate(el => {
        const prose = el.parentElement!;
        const bold = prose.querySelector('.is-bold')!;
        const italic = prose.querySelector('.is-italic')!;
        return { bold: getComputedStyle(bold).fontWeight, italic: getComputedStyle(italic).fontStyle, width: prose.clientWidth, scroll: prose.scrollWidth };
      });
      expect(Number(styles.bold)).toBeGreaterThanOrEqual(500);
      expect(styles.italic).toBe('italic');
      expect(styles.scroll).toBeLessThanOrEqual(styles.width + 1);
      await panel.locator('.agent-trace.is-live .agent-md-h').scrollIntoViewIfNeeded();
      await panel.screenshot({ path: testInfo.outputPath(`markdown-${theme}-${width}.png`) });
    }
    // Updating the same live row upgrades incomplete syntax without duplicating prose.
    await page.evaluate(() => { (window as any).PM.AgentUI.state.trace.at(-1).text = '**Stream'; });
    await expect(panel.locator('.agent-trace.is-live .agent-trace-prose').last()).toHaveText('**Stream');
    await page.evaluate(() => { (window as any).PM.AgentUI.state.trace.at(-1).text = '**Stream complete**'; });
    await expect(panel.locator('.agent-trace.is-live .agent-trace-prose').last().locator('.is-bold')).toHaveText(['Stream', ' ', 'complete']);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
