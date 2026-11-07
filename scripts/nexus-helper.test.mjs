import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import assert from 'node:assert/strict';

const target = 'https://www.nexusmods.com/bonelab/mods/12?file_id=34&nmm=1';
const script = readFileSync(new URL('../src-tauri/src/nexus_helper.js', import.meta.url), 'utf8').replace('__MM_INFO__', JSON.stringify({ url: target }));
function run({ login = false, disabled = false, text = 'Slow download', offFile = false } = {}) {
  let clicks = 0;
  let tick;
  let timeout;
  const navigations = [];
  const button = { textContent: text, disabled, getAttribute: () => null, getClientRects: () => [1], click: () => clicks++ };
  const location = {
    hostname: login ? 'users.nexusmods.com' : 'www.nexusmods.com',
    pathname: offFile ? '/' : '/bonelab/mods/12', search: '?file_id=34',
    set href(value) { navigations.push(value); }, replace: (value) => navigations.push(value),
  };
  vm.runInNewContext(script, {
    URL, URLSearchParams, location, window: {},
    document: { querySelector: () => null, querySelectorAll: () => [button] },
    setInterval: (fn) => { tick = fn; }, setTimeout: (fn) => { timeout = fn; },
    clearInterval() {}, clearTimeout() {},
  });
  return { button, navigations, tick: () => tick(), timeout: () => timeout(), clicks: () => clicks };
}
test('clicks Slow download once, even after repeated checks', () => {
  const page = run();
  page.tick(); page.tick();
  assert.equal(page.clicks(), 1);
  assert.deepEqual(page.navigations, ['mm-nexus://hide']);
});
test('waits until Slow download is enabled', () => {
  const page = run({ disabled: true });
  assert.equal(page.clicks(), 0);
  page.button.disabled = false; page.tick();
  assert.equal(page.clicks(), 1);
});
test('never selects a premium download button', () => {
  const page = run({ text: 'Fast download' });
  page.tick();
  assert.equal(page.clicks(), 0);
  page.timeout();
  assert.deepEqual(page.navigations, ['mm-nexus://show']);
});
test('reveals login once without clicking', () => {
  const page = run({ login: true });
  page.tick(); page.timeout();
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, ['mm-nexus://show']);
});
test('returns to the requested file after login', () => {
  const page = run({ offFile: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, [target]);
});
