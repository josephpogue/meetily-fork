// User-mock's hands inside a test Mac: talks W3C WebDriver to the app's embedded server.
// Exit codes: 0 done; 1 the text or window never appeared, the app answered too slowly ("app did not answer within Ns"),
// or it reported an error; 3 nothing is answering (connection refused, reset or closed); 64 usage.
import net from 'node:net';
import { remote } from 'webdriverio';

const [verb, a, b] = process.argv.slice(2);
const port = Number(process.env.TESTENV_WEBDRIVER_PORT ?? 4445);
const secs = Number(b ?? 30);
const xpathText = (t) => `//*[normalize-space(text())=${JSON.stringify(t)} or @aria-label=${JSON.stringify(t)} or @placeholder=${JSON.stringify(t)}]`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Exit 3 means only "nothing is answering": the connection was refused, reset or closed before any reply.
// A slow or hung app that does answer late is a timeout, exit 1 "app did not answer within Ns".
const CONNECTION_CODES = ['ECONNREFUSED', 'ECONNRESET', 'UND_ERR_SOCKET', 'UND_ERR_CLOSED'];
const errorCode = (e) => e?.code ?? e?.cause?.code;
const isConnectionError = (e) => CONNECTION_CODES.includes(errorCode(e));
const isTimeout = (e) => errorCode(e) === 'ETIMEDOUT';
// The app answers but has no window yet. The plugin waits 10 s for one, then replies "no such window"
// / "No window could be found" (a reply from the app, not a Node error, so the text is all there is).
const isNoWindow = (e) => /no such window|no window could be found/i.test(e?.message ?? '');

// One fixed request timeout for the whole session. webdriverio also uses it as the headers and body timeout of the
// session's connection and fixes that at the first command, so it cannot follow the time left in a `wait`.
// TESTENV_UI_TIMEOUT_SECS exists for the tests.
const TIMEOUT_MS = Number(process.env.TESTENV_UI_TIMEOUT_SECS ?? 5) * 1000;

// Is anything listening on the port? Returns a connection code if not, else null.
function probe() {
  return new Promise((resolve) => {
    const sock = net.connect({ host: '127.0.0.1', port });
    sock.setTimeout(1000, () => { sock.destroy(); resolve(null); });
    sock.once('connect', () => { sock.destroy(); resolve(null); });
    sock.once('error', (e) => resolve(CONNECTION_CODES.includes(e.code) ? e.code : null));
  });
}

// webdriverio rethrows a failed session creation as a plain Error and drops the code, so put it back:
// the code named in the message, else a refused port found by probing, else a timeout we measured ourselves.
async function withCode(e, elapsedMs) {
  if (errorCode(e) || isNoWindow(e)) return e;
  const named = /error code (\w+)/.exec(e?.message ?? '')?.[1];
  const code = named ?? (await probe()) ?? (elapsedMs >= TIMEOUT_MS - 250 ? 'ETIMEDOUT' : undefined);
  if (code) e.code = code;
  return e;
}

// A refused connection and other errors fail at once. For `wait`, no-such-window and a timed-out attempt are retried
// until `retryUntil` (ms epoch); the plugin holds POST /session up to 10 s waiting for a window.
async function connect(retryUntil) {
  for (;;) {
    const started = Date.now();
    try {
      return await remote({ hostname: '127.0.0.1', port, capabilities: {}, logLevel: 'silent', connectionRetryCount: 0, connectionRetryTimeout: TIMEOUT_MS });
    } catch (e) {
      await withCode(e, Date.now() - started);
      if (!(isNoWindow(e) || isTimeout(e)) || !retryUntil || Date.now() >= retryUntil) throw e;
      await sleep(Math.min(1000, Math.max(0, retryUntil - Date.now())));
    }
  }
}

// Poll until the text is displayed or `ms` run out. A dead or hung app propagates at once (connection error or
// timeout); any other error, such as no such element, counts as "not there yet".
async function find(browser, t, ms) {
  const end = Date.now() + ms;
  for (;;) {
    try {
      const el = await browser.$(xpathText(t));
      if (await el.isDisplayed()) return el;
    } catch (e) {
      if (isConnectionError(e) || isTimeout(e)) throw e;
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
  } catch (e) {
    if (isConnectionError(e) || isTimeout(e)) throw e; // a dead or hung app is not "execute rejected"
  } // else the add-on rejected execute
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
    if (isTimeout(e)) { console.error(`app did not answer within ${TIMEOUT_MS / 1000}s`); return 1; }
    if (verb === 'wait' && isNoWindow(e)) { console.error(`not found after ${secs}s: ${a}`); return 1; }
    console.error(e?.message ?? String(e));
    return 1;
  } finally {
    await browser?.deleteSession().catch(() => {});
  }
}

// exitCode, not process.exit(): exit would cut off piped stdout at 64 KB.
process.exitCode = await run();
