import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

test('both screens meet WCAG AA automated accessibility checks', async ({
  page,
}) => {
  await page.goto('/');
  const sync = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(
    sync.violations.map(({ id, nodes }) => ({
      id,
      nodes: nodes.map(({ target, failureSummary }) => ({
        target,
        failureSummary,
      })),
    })),
  ).toEqual([]);
  await page.getByRole('button', { name: 'Your Rooms 02' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  const rooms = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(
    rooms.violations.map(({ id, nodes }) => ({
      id,
      nodes: nodes.map(({ target, failureSummary }) => ({
        target,
        failureSummary,
      })),
    })),
  ).toEqual([]);
});
