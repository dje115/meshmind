// @ts-check
import { test, expect } from '@playwright/test';

/**
 * E2E test for the Ask page.
 * Prerequisites: node_app running at http://127.0.0.1:9900 (e.g. cargo run -p node_app)
 */
test.describe('Ask page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    // Navigate to Ask page
    await page.click('.nav-item[data-page="ask"]');
  });

  test('shows Ask page and chat input', async ({ page }) => {
    await expect(page.locator('#chat-input')).toBeVisible();
    await expect(page.locator('.chat-welcome, .chat-main')).toBeVisible();
  });

  test('can type and send a message', async ({ page }) => {
    const input = page.locator('#chat-input');
    await input.fill('How many invoices do I have?');
    await page.click('#chat-send');
    // Wait for either response bubble or error; mock backend returns quickly
    await page.waitForSelector('.chat-bubble, .chat-bubble-assistant, .chat-welcome', {
      timeout: 15000,
    });
  });
});
