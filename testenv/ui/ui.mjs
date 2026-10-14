// User-mock's hands inside a test Mac: talks W3C WebDriver to the app's embedded server.
// Exit codes: 0 done; 1 the text or window never appeared, the app answered too slowly ("app did not answer within Ns"),
// or it reported an error; 3 nothing is answering (connection refused, reset or closed); 64 usage.
import net from 'node:net';
import { remote } from 'webdriverio';

const [verb, a, b] = process.argv.slice(2);
const port = Number(process.env.TESTENV_WEBDRIVER_PORT ?? 4445);
const secs = Number(b ?? 30);
// An element matches by its visible text, its aria-label (what an icon-only button is named by) or its placeholder.
// `click` and `type` take only a whole match, trimmed and ignoring case, so `click "Start"` never presses "Get Started".
// `wait` tries that exact match first, then text that contains it. Scripts and styles carry page text but are never on screen.
const SKIP = 'not(self::script or self::style or self::title)';
const lower = (x) => `translate(normalize-space(${x}), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz')`;
const xpathExact = (t) => {
  const v = JSON.stringify(t.trim().replace(/\s+/g, ' ').toLowerCase());
  return `//*[${SKIP} and (text()[${lower('.')}=${v}] or ${lower('@aria-label')}=${v} or ${lower('@placeholder')}=${v})]`;
};
const xpathContains = (t) => `//*[${SKIP} and (text()[contains(normalize-space(.), ${JSON.stringify(t)})] or contains(@aria-label, ${JSON.stringify(t)}) or contains(@placeholder, ${JSON.stringify(t)}))]`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Exit 3 means only "nothing is answering": the connection was refused, reset or closed before any reply.
// A slow or hung app that does answer late is a timeout, exit 1 "app did not answer within Ns".
const CONNECTION_CODES = ['ECONNREFUSED', 'ECONNRESET', 'UND_ERR_SOCKET', 'UND_ERR_CLOSED'];
const errorCode = (e) => e?.code ?? e?.cause?.code;
const isConnectionError = (e) => CONNECTION_CODES.includes(errorCode(e));
const TIMEOUT_CODES = ['ETIMEDOUT', 'UND_ERR_HEADERS_TIMEOUT', 'UND_ERR_BODY_TIMEOUT'];
const isTimeout = (e) => TIMEOUT_CODES.includes(errorCode(e));
// The app answers but has no window yet. The plugin waits 10 s for one, then replies "no such window"
// / "No window could be found" (a reply from the app, not a Node error, so the text is all there is).
const isNoWindow = (e) => /no such window|no window could be found/i.test(e?.message ?? '');

// One fixed request timeout for the whole session. webdriverio also uses it as the headers and body timeout of the
// session's connection and fixes that at the first command, so it cannot follow the time left in a `wait`.
// The default is above the 10 s the plugin holds POST /session waiting for a window, so a cold-start read, click or
// type gets the plugin's own answer. TESTENV_UI_TIMEOUT_SECS exists for the tests.
const TIMEOUT_MS = Number(process.env.TESTENV_UI_TIMEOUT_SECS ?? 15) * 1000;

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

// First displayed element for an xpath, or null.
async function firstDisplayed(browser, xpath) {
  for (const el of await browser.$$(xpath)) if (await el.isDisplayed()) return el;
  return null;
}

// Poll until the text is displayed or `ms` run out. A dead app (connection error) propagates at once; any other
// error, such as no such element, counts as "not there yet". A poll that times out propagates too, except for `wait`
// (`retryTimeouts`): one slow poll must not end it, so it polls on until the deadline. At the deadline it throws the
// timeout only if no poll was ever answered (the app is hung), else it returns null (the text is not there).
// `contains` adds the contains fallback (only `wait` asks for it).
async function find(browser, t, ms, { retryTimeouts = false, contains = false } = {}) {
  const end = Date.now() + ms;
  let answered = false;
  let slow = null;
  for (;;) {
    try {
      const el = (await firstDisplayed(browser, xpathExact(t))) ?? (contains ? await firstDisplayed(browser, xpathContains(t)) : null);
      answered = true;
      if (el) return el;
    } catch (e) {
      if (isConnectionError(e) || (isTimeout(e) && !retryTimeouts)) throw e;
      if (isTimeout(e)) slow = e;
      else answered = true;
    }
    if (Date.now() >= end) { if (!answered && slow) throw slow; return null; }
    await sleep(250);
  }
}

// Only what a person sees: innerText skips <script> and <style>, which getText returns here as raw page text.
// An icon-only button has no text, so the aria-labels of the visible ones follow as "button: <label>" lines.
// Fall back to getText if the add-on rejects execute.
async function readScreen(browser) {
  try {
    const screen = await browser.execute(() => {
      const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden';
      const labels = [...document.querySelectorAll('button[aria-label], [role="button"][aria-label]')]
        .filter((el) => shown(el) && !el.innerText.trim())
        .map((el) => el.getAttribute('aria-label').trim())
        .filter(Boolean);
      return { text: document.body.innerText, buttons: [...new Set(labels)] };
    });
    if (typeof screen?.text === 'string') return [screen.text, ...screen.buttons.map((l) => `button: ${l}`)].join('\n');
  } catch (e) {
    if (isConnectionError(e) || isTimeout(e)) throw e; // a dead or hung app is not "execute rejected"
  } // else the add-on rejected execute
  return browser.$('body').getText();
}

async function run() {
  if (!['wait', 'click', 'type', 'read'].includes(verb)) { console.error('usage: ui.mjs wait|click|type|read'); return 64; }
  // A missing argument is the caller's mistake (64), not a screen that lacks the text (1).
  const USAGE = { wait: 'wait <text> [seconds]', click: 'click <text>', type: 'type <field> <text>', read: 'read' };
  const badArgs = (verb !== 'read' && !a?.trim()) || (verb === 'type' && b === undefined)
    || (verb === 'wait' && b !== undefined && !(Number(b) > 0));
  if (badArgs) { console.error(`usage: ui.mjs ${USAGE[verb]}`); return 64; }
  const deadline = Date.now() + secs * 1000;
  let browser;
  try {
    browser = await connect(verb === 'wait' ? deadline : 0);
    switch (verb) {
      case 'wait':
        if (!(await find(browser, a, deadline - Date.now(), { retryTimeouts: true, contains: true }))) { console.error(`not found after ${secs}s: ${a}`); return 1; }
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
