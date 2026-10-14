// User-mock's hands inside a test Mac: talks W3C WebDriver to the app's embedded server.
// Exit codes: 0 done, 1 the text or window never appeared (or the app reported an error), 3 nothing answering, 64 usage.
import { remote } from 'webdriverio';

const [verb, a, b] = process.argv.slice(2);
const port = Number(process.env.TESTENV_WEBDRIVER_PORT ?? 4445);
const secs = Number(b ?? 30);
const xpathText = (t) => `//*[normalize-space(text())=${JSON.stringify(t)} or @aria-label=${JSON.stringify(t)} or @placeholder=${JSON.stringify(t)}]`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// webdriverio keeps only the server's message text (it drops the W3C error code), so these checks match text.
// Nothing is answering: webdriverio's own "Failed to fetch [POST] <url>: please make sure you have a WebDriver compatible
// server running", or "Request failed with error code UND_ERR_SOCKET" and the like when the app dies with a request in flight.
const isConnectionError = (e) => /Failed to fetch \[|Request failed with error code|fetch failed|ECONNREFUSED|ECONNRESET|EHOSTUNREACH|driver is running on that address/i.test(e?.message ?? '');
// The app answers but has no window yet. The plugin waits 10 s for one, then replies "no such window" / "No window could be found".
const isNoWindow = (e) => /no such window|no window could be found/i.test(e?.message ?? '');
// webdriverio's own request timeout (the option below).
const isTimeout = (e) => /Request timed out/i.test(e?.message ?? '');

const REQUEST_TIMEOUT_MS = 15000; // a hung app must not hold a command for webdriverio's 120 s default

// A refused connection or any other error fails at once; no-such-window is retried until `retryUntil` (ms epoch).
// Each attempt's request timeout is the time left (the plugin can hold POST /session for 10 s), so `wait` overshoots by under a second.
async function connect(retryUntil) {
  for (;;) {
    const left = retryUntil - Date.now();
    try {
      const browser = await remote({
        hostname: '127.0.0.1', port, capabilities: {}, logLevel: 'silent', connectionRetryCount: 0,
        connectionRetryTimeout: retryUntil ? Math.max(500, left) : REQUEST_TIMEOUT_MS,
      });
      browser.options.connectionRetryTimeout = REQUEST_TIMEOUT_MS; // read per command, so later commands get the normal cap
      return browser;
    } catch (e) {
      if (!isNoWindow(e) || !retryUntil || Date.now() >= retryUntil) throw e;
      await sleep(Math.min(1000, Math.max(0, retryUntil - Date.now())));
    }
  }
}

// Poll until the text is displayed or `ms` run out. A connection error propagates at once (the app died);
// any other error, such as no such element, counts as "not there yet".
async function find(browser, t, ms) {
  const end = Date.now() + ms;
  for (;;) {
    try {
      const el = await browser.$(xpathText(t));
      if (await el.isDisplayed()) return el;
    } catch (e) {
      if (isConnectionError(e)) throw e;
    }
    if (Date.now() >= end) return null;
    await sleep(250);
  }
}

// Only what a person sees: innerText skips <script> and <style>, which getText returns here as raw page text.
// Fall back to getText if the add-on rejects execute.
async function readScreen(browser) {
  try {
    const text = await browser.execute(() => document.body.innerText);
    if (typeof text === 'string') return text;
  } catch { /* add-on rejected execute */ }
  return browser.$('body').getText();
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
      case 'read': console.log(await readScreen(browser)); break;
    }
    return 0;
  } catch (e) {
    if (isConnectionError(e)) { console.error(`app WebDriver not answering on 127.0.0.1:${port} (is the app running?)`); return 3; }
    if (verb === 'wait' && (isNoWindow(e) || (isTimeout(e) && Date.now() >= deadline))) { console.error(`not found after ${secs}s: ${a}`); return 1; }
    console.error(e?.message ?? String(e));
    return 1;
  } finally {
    await browser?.deleteSession().catch(() => {});
  }
}

// exitCode, not process.exit(): exit would cut off piped stdout at 64 KB.
process.exitCode = await run();
