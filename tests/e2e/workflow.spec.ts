import { expect, test } from '@playwright/test';

test('browser preview: save, isolate draft, discard, and label native sync unavailable', async ({
  page,
}) => {
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Start Sync' })).toBeDisabled();
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  await page.getByLabel('Name', { exact: true }).fill('Desk left');
  await page.getByRole('button', { name: 'Light Bar', exact: true }).click();
  await page.getByLabel('Left / right coordinate', { exact: true }).fill('-2');
  await page.getByRole('tab', { name: 'Height' }).click();
  await page.getByLabel('Height coordinate', { exact: true }).fill('2.2');
  await page.getByRole('tab', { name: 'Location' }).click();
  await expect(
    page.getByLabel('Left / right coordinate', { exact: true }),
  ).toHaveValue('-2');
  await page.getByRole('button', { name: /Save Room/ }).click();
  await expect(page.getByRole('status')).toHaveText('✓ PREVIEW UPDATED');
  await page.getByLabel('Name', { exact: true }).fill('Uncommitted name');
  await page.getByRole('button', { name: 'Sync' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.getByRole('button', { name: 'Stay', exact: true }).click();
  await expect(page.getByLabel('Name', { exact: true })).toHaveValue(
    'Uncommitted name',
  );
  await page.getByRole('button', { name: 'Sync' }).click();
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(
    page.getByRole('group', { name: 'Saved virtual lights' }),
  ).toContainText('Desk left');
  await expect(page.getByRole('button', { name: 'Start Sync' })).toBeDisabled();
  await expect(page.getByText(/Sync requires the desktop app/)).toBeVisible();
  await page.getByRole('button', { name: 'Punch intensity' }).click();
  await expect(
    page.getByRole('button', { name: 'Punch intensity' }),
  ).toHaveAttribute('aria-pressed', 'true');
});

test('browser preview: overflowing cards, keyboard selection and zero-brightness calibration', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  for (let index = 0; index < 9; index++)
    await page.getByRole('button', { name: 'Add virtual light' }).click();
  const row = page.getByRole('group', { name: 'Virtual lights', exact: true });
  await expect(row.getByRole('button')).toHaveCount(9);
  await expect(
    page.getByRole('button', { name: 'Select Light 9', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  expect(
    await row.evaluate(
      (element) =>
        element.scrollWidth > element.clientWidth && element.scrollLeft > 0,
    ),
  ).toBe(true);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  const first = page.getByRole('button', {
    name: 'Select Light 1',
    exact: true,
  });
  await first.focus();
  await page.keyboard.press('Enter');
  await expect(first).toHaveAttribute('aria-pressed', 'true');
  await expect(first).toBeInViewport();
  await page.getByRole('button', { name: /Save Room/ }).click();
  await page.getByRole('button', { name: 'Sync' }).click();
  await page.getByLabel('Brightness', { exact: true }).fill('0');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await expect(first).toHaveAttribute('style', /--light-color: rgb\((?!0 0 0)/);
  await page.getByRole('button', { name: 'Clear light selection' }).click();
  await expect(first).toHaveAttribute(
    'style',
    /--light-color: rgb\(179 186 179\)/,
  );
  await expect(first).toBeVisible();
  await expect(page.getByRole('status')).not.toContainText('UNSAVED');
  await page.setViewportSize({ width: 1000, height: 720 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page
    .getByRole('button', { name: 'Select Light 9', exact: true })
    .focus();
  await page.keyboard.press('Space');
  await expect(
    page.getByRole('button', { name: 'Select Light 9', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
});

test('browser preview: renders WebGL and orbits without changing room data', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  await expect(page.locator('canvas')).toBeVisible();
  await expect(page.getByText('3D view unavailable')).not.toBeVisible();
  expect(
    await page
      .locator('canvas')
      .evaluate(
        (element) => !!(element as HTMLCanvasElement).getContext('webgl2'),
      ),
  ).toBe(true);
  await page.getByRole('button', { name: /Save Room/ }).click();
  await page.getByRole('button', { name: 'Clear light selection' }).click();
  const canvas = page.locator('canvas');
  const baseline = await canvas.screenshot();
  const bounds = (await canvas.boundingBox())!;
  const start = {
    x: bounds.x + bounds.width * 0.82,
    y: bounds.y + bounds.height * 0.68,
  };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(start.x + 55, start.y - 35, { steps: 8 });
  await page.mouse.up();
  expect((await canvas.screenshot()).equals(baseline)).toBe(false);
  await expect(page.getByRole('status')).not.toContainText('UNSAVED');
  await page
    .getByRole('button', { name: 'Select Light 1', exact: true })
    .click();
  await expect(
    page.getByLabel('Left / right coordinate', { exact: true }),
  ).toHaveValue('-1.7');
  await page.getByRole('button', { name: 'Reset room view' }).click();
  await expect(page.getByRole('status')).not.toContainText('UNSAVED');
  expect(errors).toEqual([]);
});
