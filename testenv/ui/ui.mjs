// User-mock's hands inside a test Mac: talks W3C WebDriver to the app's embedded server.
// Exit codes: 0 done, 1 the text or window never appeared (or the app reported an error), 3 nothing answering, 64 usage.
import { remote } from 'webdriverio';

const [verb, a, b] = process.argv.slice(2);
const port = Number(process.env.TESTENV_WEBDRIVER_PORT ?? 4445);
const secs = Number(b ?? 30);
const xpathText = (t) => `//*[normalize-space(text())=${JSON.stringify(t)} or @aria-label=${JSON.stringify(t)} or @placeholder=${JSON.stringify(t)}]`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// webdriverio keeps only the server's message text (it drops the W3C error code), so both checks match text.
// Nothing is listening: webdriverio says "Failed to fetch ... make sure you have a WebDriver compatible server running".
const isConnectionError = (e) => /Failed to fetch|fetch failed|ECONNREFUSED|ECONNRESET|EHOSTUNREACH|driver is running on that address/i.test(e?.message ?? '');
// The app answers but has no window yet. The plugin waits 10 s for one, then replies "no such window" / "No window could be found".
const isNoWindow = (e) => /no such window|no window could be found/i.test(e?.message ?? '');

// A refused connection or any other error fails at once; no-such-window is retried until `retryUntil` (ms epoch).
async function connect(retryUntil) {
  for (;;) {
    try {
      return await remote({ hostname: '127.0.0.1', port, capabilities: {}, logLevel: 'silent', connectionRetryCount: 0 });
    } catch (e) {
      if (!isNoWindow(e) || Date.now() >= retryUntil) throw e;
      await sleep(1000);
    }
  }
}

async function find(browser, t, ms) {
  const el = await browser.$(xpathText(t));
  if (ms > 0) await el.waitForDisplayed({ timeout: ms }).catch(() => {});
  return (await el.isDisplayed().catch(() => false)) ? el : null;
}

async function run() {
  if (!['wait', 'click', 'type', 'read'].includes(verb)) { console.error('usage: ui.mjs wait|click|type|read'); return 64; }
  const deadline = Date.now() + secs * 1000;
  let browser;
  try {
    browser = await connect(verb === 'wait' ? deadline : 0);
    switch (verb) {
      case 'wait':
        if (!(await find(browser, a, deadline - Date.now()))) { console.error(`not found after ${secs}s: ${a}`); return 1; }
        break;
      case 'click': { const el = await find(browser, a, 10000); if (!el) { console.error(`nothing to click: ${a}`); return 1; } await el.click(); break; }
      case 'type': { const el = await find(browser, a, 10000); if (!el) { console.error(`no field: ${a}`); return 1; } await el.setValue(b); break; }
      case 'read': console.log(await browser.$('body').getText()); break;
    }
    return 0;
  } catch (e) {
    if (isConnectionError(e)) { console.error(`app WebDriver not answering on 127.0.0.1:${port} (is the app running?)`); return 3; }
    if (verb === 'wait' && isNoWindow(e)) { console.error(`not found after ${secs}s: ${a}`); return 1; }
    console.error(e?.message ?? String(e));
    return 1;
  } finally {
    await browser?.deleteSession().catch(() => {});
  }
}

process.exit(await run());
