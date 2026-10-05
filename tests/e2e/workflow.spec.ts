import { expect, test, type Locator, type Page } from '@playwright/test';

// Orbit damping keeps rendering briefly; wait for two identical frames.
async function settled(canvas: Locator) {
  let previous = await canvas.screenshot();
  for (let attempt = 0; attempt < 30; attempt += 1) {
    await canvas.page().waitForTimeout(150);
    const next = await canvas.screenshot();
    if (next.equals(previous)) return next;
    previous = next;
  }
  throw new Error('The room view did not settle.');
}

// Fraction of visibly changed pixels; overlaid buttons alone stay far below
// the threshold used for a changed camera.
function changed(page: Page, first: Buffer, second: Buffer) {
  return page.evaluate(
    async ([a, b]) => {
      const pixels = async (data: string) => {
        const bitmap = await createImageBitmap(
          await (await fetch(`data:image/png;base64,${data}`)).blob(),
        );
        const context = new OffscreenCanvas(
          bitmap.width,
          bitmap.height,
        ).getContext('2d')!;
        context.drawImage(bitmap, 0, 0);
        return context.getImageData(0, 0, bitmap.width, bitmap.height).data;
      };
      const [x, y] = await Promise.all([pixels(a), pixels(b)]);
      let count = 0;
      for (let index = 0; index < x.length; index += 4) {
        const delta =
          Math.abs(x[index] - y[index]) +
          Math.abs(x[index + 1] - y[index + 1]) +
          Math.abs(x[index + 2] - y[index + 2]);
        if (delta > 24) count += 1;
      }
      return count / (x.length / 4);
    },
    [first.toString('base64'), second.toString('base64')],
  );
}

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

test('coordinate inputs accept keystroke typing and clamp arrow steps', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  await page.getByRole('tab', { name: 'Height' }).click();
  const height = page.getByLabel('Height coordinate', { exact: true });
  await height.selectText();
  await height.pressSequentially('0.5');
  await expect(height).toHaveValue('0.5');
  await height.press('Tab');
  await expect(height).toHaveValue('0.5');
  await height.selectText();
  await height.pressSequentially('2.99');
  await height.press('ArrowUp');
  await height.press('ArrowUp');
  await expect(height).toHaveValue('3');
  await page.getByRole('tab', { name: 'Location' }).click();
  const x = page.getByLabel('Left / right coordinate', { exact: true });
  await x.selectText();
  await x.pressSequentially('-1.5');
  await expect(x).toHaveValue('-1.5');
  await x.press('Enter');
  await expect(x).toHaveValue('-1.5');
  await page.getByRole('tab', { name: 'Height' }).click();
  await expect(height).toHaveValue('3');
});

test('Save shortcut stores coordinate text that is still being typed, clamped', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  const save = page.getByRole('button', { name: /Save Room/ });
  await save.click();
  await expect(save).toBeDisabled();
  const x = page.getByLabel('Left / right coordinate', { exact: true });
  await x.selectText();
  await x.pressSequentially('5');
  await expect(save).toBeEnabled();
  await expect(page.getByRole('status')).toContainText('UNSAVED');
  await x.press('ControlOrMeta+s');
  await expect(page.getByRole('status')).toHaveText('✓ PREVIEW UPDATED');
  await expect(x).toHaveValue('3');
  await x.press('Tab');
  await expect(x).toHaveValue('3');
  await expect(save).toBeDisabled();
  await expect(page.getByRole('status')).not.toContainText('UNSAVED');
});

test('browser preview: resizing keeps the orbited room view until Reset view', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  await page.getByRole('button', { name: 'Clear light selection' }).click();
  const canvas = page.locator('canvas');
  await expect(canvas).toBeVisible();
  const before = (await canvas.boundingBox())!;
  const start = {
    x: before.x + before.width * 0.82,
    y: before.y + before.height * 0.68,
  };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(start.x + 90, start.y - 45, { steps: 10 });
  await page.mouse.up();
  await settled(canvas);
  await page.setViewportSize({ width: 1120, height: 820 });
  await expect
    .poll(async () => (await canvas.boundingBox())!.width)
    .not.toBe(before.width);
  const resized = await settled(canvas);
  await page.getByRole('button', { name: 'Reset room view' }).click();
  const reset = await settled(canvas);
  expect(await changed(page, resized, reset)).toBeGreaterThan(0.05);
  expect(errors).toEqual([]);
});

test('browser preview: Reset view is exact after orbiting into a limit', async ({
  page,
}) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Your Rooms' }).click();
  await page.getByRole('button', { name: 'Add virtual light' }).click();
  await page.getByRole('button', { name: 'Clear light selection' }).click();
  const canvas = page.locator('canvas');
  await expect(canvas).toBeVisible();
  const bounds = (await canvas.boundingBox())!;
  const start = {
    x: bounds.x + bounds.width * 0.82,
    y: bounds.y + bounds.height * 0.68,
  };
  const orbit = async (dx: number, dy: number) => {
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(start.x + dx, start.y + dy, { steps: 8 });
    await page.mouse.up();
    // A camera clamped at a limit looks settled while damping momentum is
    // still draining; wait out the scene's 120 settle frames by counting
    // animation frames, so slow CI renderers get the same drain.
    await page.evaluate(
      () =>
        new Promise<void>((resolve) => {
          let frames = 0;
          const tick = () =>
            ++frames >= 130 ? resolve() : requestAnimationFrame(tick);
          requestAnimationFrame(tick);
        }),
    );
    return settled(canvas);
  };
  const reset = async () => {
    await page.getByRole('button', { name: 'Reset room view' }).click();
    // Keep the hovered button out of the compared pixels.
    await page.mouse.move(0, 0);
    return settled(canvas);
  };
  const initial = await reset();
  // A small orbit from the default pose is the reference for leaked motion.
  const nudged = await orbit(40, -20);
  expect(await changed(page, initial, nudged)).toBeGreaterThan(0.05);
  expect(await changed(page, initial, await reset())).toBe(0);
  // Drags far past the azimuth and polar limits leave damping momentum that
  // cannot move the clamped camera; Reset must still restore the exact pose.
  for (const [dx, dy] of [
    [300, 0],
    [120, 0],
    [0, 200],
    [-300, 0],
  ]) {
    await orbit(dx, dy);
    expect(await changed(page, initial, await reset())).toBe(0);
  }
  expect(await changed(page, nudged, await orbit(40, -20))).toBe(0);
  // Momentum left at a limit drains instead of resisting the next orbit, so
  // the same reverse orbit from the limit lands identically after any overshoot.
  await reset();
  await orbit(300, 0);
  const reversed = await orbit(-200, 0);
  await reset();
  await orbit(900, 0);
  expect(await changed(page, reversed, await orbit(-200, 0))).toBe(0);
  expect(errors).toEqual([]);
});
