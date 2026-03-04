/** @type {import('@playwright/test').PlaywrightTestConfig} */
export default {
  testDir: './e2e',
  use: {
    baseURL: 'http://127.0.0.1:9900',
    trace: 'on-first-retry',
  },
  webServer: undefined, // Assume node_app is already running; set to start it if desired
  timeout: 15000,
};
