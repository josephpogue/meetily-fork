// User-mock's hands inside a test Mac: talks W3C WebDriver to the app's embedded server.
import { remote } from 'webdriverio';

const [verb, a, b] = process.argv.slice(2);
const xpathText = (t) => `//*[normalize-space(text())=${JSON.stringify(t)} or @aria-label=${JSON.stringify(t)} or @placeholder=${JSON.stringify(t)}]`;

let browser;
try {
  browser = await remote({ hostname: '127.0.0.1', port: 4445, capabilities: {}, logLevel: 'error', connectionRetryCount: 1 });
} catch {
  console.error('app WebDriver not answering on 127.0.0.1:4445 (is the app running?)');
  process.exit(3);
}

async function find(t, secs) {
  const el = await browser.$(xpathText(t));
  await el.waitForDisplayed({ timeout: secs * 1000 }).catch(() => {});
  return (await el.isDisplayed().catch(() => false)) ? el : null;
}

const secs = Number(b ?? 30);
switch (verb) {
  case 'wait': if (!(await find(a, secs))) { console.error(`not found after ${secs}s: ${a}`); process.exit(1); } break;
  case 'click': { const el = await find(a, 10); if (!el) { console.error(`nothing to click: ${a}`); process.exit(1); } await el.click(); break; }
  case 'type': { const el = await find(a, 10); if (!el) { console.error(`no field: ${a}`); process.exit(1); } await el.setValue(b); break; }
  case 'read': console.log(await browser.$('body').getText()); break;
  default: console.error('usage: ui.mjs wait|click|type|read'); process.exit(64);
}
