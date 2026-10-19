import { expect, test } from "@playwright/test";

for (const cores of [32, 64, 96, 128]) {
  // The appearance setting supports 12–24px; 16px is the default.
  for (const fontSize of [16, 24]) {
    test(`${cores} cores at ${fontSize}px expand without clipping and remain scrollable`, async ({
      page,
    }) => {
      await page.goto(
        `/e2e/fixtures/resource-monitor.html?cores=${cores}&fontSize=${fontSize}`,
        { waitUntil: "domcontentloaded" },
      );
      const toggle = page.getByRole("button", {
        name: `${cores} CPU`,
        exact: true,
      });
      const lastCore = page.getByText(`cpu${cores - 1}`, { exact: true });
      const firstCore = page.getByText("cpu0", { exact: true });
      // Find the clipping element from the content, independently of the animation implementation.
      const clip = lastCore.locator(
        "xpath=ancestor::div[contains(@class, 'overflow-hidden')][1]",
      );
      const scroll = page.locator(".terminal-scroll");
      const memory = page
        .getByText("Memory", { exact: true })
        .locator(
          "xpath=ancestor::div[contains(@class, 'nyaterm-wallpaper-card')][1]",
        );

      await expect(page.getByText(/^cpu\d+$/)).toHaveCount(cores);
      await expect
        .poll(() => clip.evaluate((element) => element.clientHeight))
        .toBe(0);
      await toggle.click();

      // JSDOM cannot detect this: all core rows can exist in DOM while the last ones are clipped.
      await expect
        .poll(() =>
          clip.evaluate(
            (element) => element.scrollHeight - element.clientHeight,
          ),
        )
        .toBeLessThanOrEqual(1);
      await expect
        .poll(async () => {
          const last = await lastCore.locator("..").boundingBox();
          const card = await memory.boundingBox();
          return card.y - (last.y + last.height);
        })
        .toBeGreaterThan(0);

      await lastCore.scrollIntoViewIfNeeded();
      await expect(lastCore).toBeInViewport({ ratio: 1 });
      await expect
        .poll(() => scroll.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(0);
      await memory.scrollIntoViewIfNeeded();
      await expect(memory).toBeInViewport({ ratio: 1 });
      // Scrolling belongs to the panel, not the whole document.
      expect(
        await page.evaluate(() => document.documentElement.scrollHeight),
      ).toBe(600);

      await toggle.click();
      await expect
        .poll(() => clip.evaluate((element) => element.clientHeight))
        .toBe(0);
      await toggle.click();
      await expect
        .poll(() =>
          clip.evaluate(
            (element) => element.scrollHeight - element.clientHeight,
          ),
        )
        .toBeLessThanOrEqual(1);
      await firstCore.scrollIntoViewIfNeeded();
      await expect(firstCore).toBeInViewport({ ratio: 1 });
      await lastCore.scrollIntoViewIfNeeded();
      await expect(lastCore).toBeInViewport({ ratio: 1 });
    });
  }
}
