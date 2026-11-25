// Unit rows for the pure filter: hostile spellings, no proxy involved. Each
// refused row has an allow control (the same request with the placeholder only),
// so a refusal cannot be a side effect of a wrong host or a missing field.
import test from 'node:test';
import assert from 'node:assert/strict';
import { makeFilter, fold } from '../filter.mjs';

const P = 'fake_value_0f0e0d0c-aaaa-4bbb-8ccc-123456789abc';
const HOST = 'provider.test';

function filter(extra = {}) {
  return makeFilter({ providerHosts: [HOST], placeholders: [P], ...extra });
}

// Builds the (request, info) pair the proxy would pass. `headers` is a flat
// name,value list so duplicates and odd spellings survive, as in rawHeaders.
function req({ headers = [], target = '/v1/messages', body = null, host = HOST, scheme = 'https', ambiguous = false, method = 'POST' } = {}) {
  const request = new Request(`https://${host}${target}`, { method, body, headers: { 'x-ignore': '1' } });
  const info = {
    scheme,
    target: { host, port: 443 },
    requestTarget: target,
    normalizedRequestTarget: target,
    requestTargetIsAmbiguous: ambiguous,
    rawHeaders: ['Host', host, ...headers],
  };
  return [request, info];
}

const bearer = ['Authorization', `Bearer ${P}`];

test('control: the placeholder in Authorization is allowed', async () => {
  const d = await filter()(...req({ headers: bearer }));
  assert.equal(d.action, 'allow');
});

test('control: the placeholder in x-api-key is allowed', async () => {
  const d = await filter()(...req({ headers: ['x-api-key', P] }));
  assert.equal(d.action, 'allow');
});

const refused = [
  ['no credential at all', []],
  ['a foreign bearer token', ['Authorization', 'Bearer sk-attacker-key-0000']],
  ['a foreign x-api-key', ['x-api-key', 'sk-attacker-key-0000']],
  ['placeholder plus a foreign key in another header', [...bearer, 'x-api-key', 'sk-attacker-key-0000']],
  ['two Authorization headers', [...bearer, 'Authorization', 'Bearer sk-attacker-key-0000']],
  ['the same placeholder twice', [...bearer, 'Authorization', `Bearer ${P}`]],
  ['underscore spelling X_Api_Key', ['X_Api_Key', 'sk-attacker-key-0000']],
  ['dotted spelling x.api.key next to the placeholder', [...bearer, 'x.api.key', 'sk-attacker-key-0000']],
  ['placeholder with trailing text', ['Authorization', `Bearer ${P}x`]],
  ['placeholder with a second token', ['Authorization', `Bearer ${P} sk-attacker-key-0000`]],
  ['placeholder prefix only', ['Authorization', `Bearer ${P.slice(0, 20)}`]],
  ['placeholder in a custom header, foreign key in Authorization', ['x-foo', P, 'Authorization', 'Bearer sk-attacker-key-0000']],
  ['placeholder in a custom header next to a valid credential', [...bearer, 'x-foo', P]],
  ['placeholder in a cookie next to a valid credential', [...bearer, 'cookie', `a=${P}`]],
];
for (const [name, headers] of refused) {
  test(`refused: ${name}`, async () => {
    const d = await filter()(...req({ headers }));
    assert.equal(d.action, 'deny');
    assert.equal(d.mark, 's11-filter');
  });
}

test('refused: placeholder in the query string', async () => {
  const d = await filter()(...req({ headers: bearer, target: `/v1/messages?note=${P}` }));
  assert.equal(d.action, 'deny');
  assert.match(d.reason, /request target/);
});

test('refused: placeholder in the query, percent-encoded', async () => {
  const enc = P.replace('fake', '%66ake');
  const d = await filter()(...req({ headers: bearer, target: `/v1/messages?note=${enc}` }));
  assert.equal(d.action, 'deny');
});

test('refused: placeholder in the body', async () => {
  const d = await filter()(...req({ headers: bearer, body: JSON.stringify({ messages: [{ role: 'user', content: `say ${P}` }] }) }));
  assert.equal(d.action, 'deny');
  assert.match(d.reason, /body/);
});

test('control: a body without the placeholder is allowed', async () => {
  const d = await filter()(...req({ headers: bearer, body: JSON.stringify({ messages: [{ role: 'user', content: 'hi' }] }) }));
  assert.equal(d.action, 'allow');
});

test('refused: a body over the scan limit', async () => {
  const d = await filter({ bodyLimit: 16 })(...req({ headers: bearer, body: 'x'.repeat(64) }));
  assert.equal(d.action, 'deny');
  assert.match(d.reason, /too large/);
});

test('refused: placeholder split across body chunks', async () => {
  const a = P.slice(0, 17);
  const b = P.slice(17);
  const stream = new ReadableStream({
    start(c) {
      c.enqueue(new TextEncoder().encode(`{"x":"${a}`));
      c.enqueue(new TextEncoder().encode(`${b}"}`));
      c.close();
    },
  });
  const request = new Request(`https://${HOST}/v1/messages`, { method: 'POST', body: stream, duplex: 'half' });
  const info = { scheme: 'https', target: { host: HOST, port: 443 }, requestTarget: '/v1/messages', rawHeaders: ['Host', HOST, ...bearer] };
  const d = await filter()(request, info);
  assert.equal(d.action, 'deny');
});

test('refused: another host', async () => {
  const d = await filter()(...req({ headers: bearer, host: 'elsewhere.test' }));
  assert.equal(d.action, 'deny');
});

test('refused: plain http to the provider host', async () => {
  const d = await filter()(...req({ headers: bearer, scheme: 'http' }));
  assert.equal(d.action, 'deny');
});

test('refused: an ambiguous request target', async () => {
  const d = await filter()(...req({ headers: bearer, ambiguous: true }));
  assert.equal(d.action, 'deny');
});

test('refused: missing info fails closed', async () => {
  const d = await filter()(new Request(`https://${HOST}/`));
  assert.equal(d.action, 'deny');
});

test('refused: a GET that declares a body the filter cannot read', async () => {
  const [request, info] = req({ headers: bearer, method: 'GET' });
  info.unshownBody = async () => true;
  const d = await filter()(request, info);
  assert.equal(d.action, 'deny');
});

test('fold: spellings of one header name are one name', () => {
  assert.equal(fold('X_Api_Key'), 'x-api-key');
  assert.equal(fold('x.api.key'), 'x-api-key');
  assert.equal(fold('X-API-KEY'), 'x-api-key');
});

test('construction: a short placeholder is refused', () => {
  assert.throws(() => makeFilter({ providerHosts: [HOST], placeholders: ['abc'] }));
});

test('overwrite mode: allows with the agent credentials removed and ours set', async () => {
  const f = filter({ mode: 'overwrite', overwrite: { headers: [['Authorization', 'Bearer FAKE-REAL']] } });
  const d = await f(...req({ headers: ['x_api_key', 'sk-attacker-key-0000', 'Authorization', 'Bearer sk-attacker-key-0000'] }));
  assert.equal(d.action, 'allow');
  assert.ok(d.removeHeaders.includes('x-api-key'));
  assert.deepEqual(d.setHeaders, [['Authorization', 'Bearer FAKE-REAL']]);
});

test('overwrite mode: still refuses the placeholder in a body', async () => {
  const f = filter({ mode: 'overwrite', overwrite: { headers: [['Authorization', 'Bearer FAKE-REAL']] } });
  const d = await f(...req({ body: `{"a":"${P}"}` }));
  assert.equal(d.action, 'deny');
});

test('placeholders may be read late, and none registered refuses everything', async () => {
  let live = [];
  const f = makeFilter({ providerHosts: [HOST], placeholders: () => live });
  assert.equal((await f(...req({ headers: bearer }))).action, 'deny');
  live = [P];
  assert.equal((await f(...req({ headers: bearer }))).action, 'allow');
});
