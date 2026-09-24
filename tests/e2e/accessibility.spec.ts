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
  const addPhysical = page.getByRole('button', { name: 'Add physical light' });
  await addPhysical.click();
  const dialog = page.getByRole('dialog', { name: 'Add a physical light' });
  await expect(dialog).toContainText(
    'Physical lights require the desktop app.',
  );
  await expect(
    page.getByRole('button', { name: 'Close physical lights' }),
  ).toBeFocused();
  const discovery = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(discovery.violations).toEqual([]);
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await expect(addPhysical).toBeFocused();
  await expect(
    page
      .getByRole('group', { name: 'Virtual lights', exact: true })
      .getByRole('button'),
  ).toHaveCount(1);
});
