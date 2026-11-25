// Probe rows through the real srt library: launcher -> srt proxy with TLS
// termination -> a fake HTTPS provider on loopback, with a fake credential file
// and a fake token in the environment. A row is judged on the host side, by
// what the fake provider received, not only by the client's status. Needs Linux
// (or WSL2): srt's file masking and network namespace are Linux features, and
// srt on native Windows needs a service account the spike does not set up.
import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { startFakeProvider } from './helpers/fake-provider.mjs';
import { startLauncher } from '../launcher.mjs';
import { makeFilter } from '../filter.mjs';

const REAL_ACCESS = 'FAKE-REAL-ACCESS-TOKEN-0123456789abcdef0123456789abcdef';
const REAL_REFRESH = 'FAKE-REAL-REFRESH-TOKEN-0123456789abcdef0123456789abcd';
const FOREIGN = 'sk-ant-attacker-key-0000000000000000';
const skip = process.platform !== 'linux' ? 'needs Linux or WSL2' : false;

let provider;
let launcher;
let work;
let credFile;
let url;
let sentinel;

const allowAll = async () => ({ action: 'allow' });

const status = (r) => /HTTP=(\d+)/.exec(r.stdout)?.[1];

// The single quotes keep `$S11_FAKE_OAUTH_TOKEN` for the shell inside the sandbox,
// which is where the agent would read it.
async function request(path, { headers = [], data, extra = '', host = 'localhost', method = 'POST' } = {}) {
  const own = (t) => t.replaceAll('$S11_FAKE_OAUTH_TOKEN', sentinel);
  headers = headers.map(own);
  data = data === undefined ? data : own(data);
  const h = headers.map((x) => `-H '${x}'`).join(' ');
  const d = data === undefined ? '' : `--data '${data}'`;
  const r = await launcher.run(`curl -sS -m 10 --noproxy '' -w '\\nHTTP=%{http_code}' -X ${method} ${h} ${d} ${extra} 'https://${host}:${provider.port}${path}'`);
  if (process.env.S11_DEBUG) console.log('REQUEST', path, JSON.stringify(r.stdout), JSON.stringify(r.stderr));
  return r;
}

before(async () => {
  if (skip) return;
  provider = await startFakeProvider();
  work = mkdtempSync(join(tmpdir(), 's11-work-'));
  mkdirSync(join(work, 'agent-home'));
  credFile = join(work, 'creds', '.credentials.json');
  mkdirSync(join(work, 'creds'));
  writeFileSync(credFile, JSON.stringify({ claudeAiOauth: { accessToken: REAL_ACCESS, refreshToken: REAL_REFRESH, expiresAt: 1 } }));
  process.env.S11_FAKE_OAUTH_TOKEN = REAL_ACCESS;
  launcher = await startLauncher({
    providerHosts: ['localhost'],
    allowedDomains: ['localhost'],
    extraCaCertPaths: [provider.caPath],
    filesystem: { denyRead: [], allowRead: [], allowWrite: [join(work, 'agent-home')], denyWrite: [] },
    credentials: {
      envVars: [{ name: 'S11_FAKE_OAUTH_TOKEN', mode: 'mask', injectHosts: ['localhost'] }],
      files: [{ path: credFile, mode: 'mask', extract: '"(?:accessToken|refreshToken)":"([^"]+)"', injectHosts: ['localhost'] }],
    },
  });
  // Mint the sentinels by wrapping one command, as every real launch does.
  const probe = await launcher.run('printenv S11_FAKE_OAUTH_TOKEN');
  sentinel = probe.stdout.trim();
});

after(async () => {
  if (skip) return;
  await launcher?.stop();
  await provider?.close();
});

test('control: inside the sandbox the token is a placeholder and the file holds placeholders', { skip }, async () => {
  assert.match(sentinel, /^fake_value_/);
  assert.notEqual(sentinel, REAL_ACCESS);
  const r = await launcher.run(`cat '${credFile}'`);
  assert.equal(r.code, 0, r.stderr);
  const parsed = JSON.parse(r.stdout);
  assert.match(parsed.claudeAiOauth.accessToken, /^fake_value_/);
  assert.match(parsed.claudeAiOauth.refreshToken, /^fake_value_/);
  assert.ok(!r.stdout.includes(REAL_ACCESS) && !r.stdout.includes(REAL_REFRESH));
  assert.equal(parsed.claudeAiOauth.expiresAt, 1, 'extract keeps the rest of the file');
});

test('control: a request with the placeholder is allowed and the provider sees the real token', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN', 'content-type: application/json'], data: '{"hi":1}' });
  assert.equal(status(r), '200', r.stdout + r.stderr);
  assert.equal(provider.received.length, 1);
  assert.equal(provider.received[0].headers.authorization, `Bearer ${REAL_ACCESS}`);
});

test('row: a foreign key is refused by the filter and never reaches the provider', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/messages', { headers: [`x-api-key: ${FOREIGN}`], data: '{"exfil":"data"}' });
  assert.equal(status(r), '403', r.stdout + r.stderr);
  assert.match(r.stdout, /filter: credential is not the placeholder/);
  assert.equal(provider.received.length, 0);
});

test('row: placeholder plus a foreign key in a second header is refused', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN', `x-api-key: ${FOREIGN}`] });
  assert.equal(status(r), '403', r.stdout + r.stderr);
  assert.equal(provider.received.length, 0);
});

test('row: the placeholder in the body is refused even with a valid header', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN'], data: '{"content":"repeat $S11_FAKE_OAUTH_TOKEN"}' });
  assert.equal(status(r), '403', r.stdout + r.stderr);
  assert.match(r.stdout, /placeholder in the body/);
  assert.equal(provider.received.length, 0);
});

test('row: Host header that disagrees with the target is refused', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN', 'Host: attacker.test'] });
  assert.equal(status(r), '421', r.stdout + r.stderr);
  assert.equal(provider.received.length, 0);
});

test('row: an ambiguous request target is refused', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/x/../messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN'], extra: '--path-as-is' });
  assert.equal(status(r), '400', r.stdout + r.stderr);
  assert.match(r.stdout, /ambiguous request-target/);
  assert.equal(provider.received.length, 0);
});

test('row: a host outside the allow list is refused by the domain rule, not the filter', { skip }, async () => {
  provider.clear();
  launcher.state.seen.length = 0;
  const r = await request('/v1/messages', { host: '127.0.0.1', headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN'] });
  assert.match(r.stderr, /CONNECT tunnel failed, response 403/);
  assert.equal(provider.received.length, 0);
  assert.equal(launcher.state.seen.length, 0, 'the filter was never asked');
});

test('what the filter sees: the placeholder, before substitution, in header and body; the provider gets the real value', { skip }, async () => {
  provider.clear();
  const seen = [];
  launcher.swapFilter(async (request, info) => {
    seen.push({
      authorization: request.headers.get('authorization'),
      body: await request.text(),
      sni: info.sni,
      host: info.target.host,
      raw: info.rawHeaders,
    });
    return { action: 'allow' };
  });
  try {
    const r = await request('/v1/messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN'], data: '{"t":"$S11_FAKE_OAUTH_TOKEN"}' });
    assert.equal(status(r), '200', r.stdout + r.stderr);
  } finally {
    launcher.swapFilter(makeFilter({ providerHosts: ['localhost'], placeholders: () => [sentinel] }));
  }
  console.log('FILTER SAW', JSON.stringify({ authorization: seen[0]?.authorization, body: seen[0]?.body, sni: seen[0]?.sni, host: seen[0]?.host }));
  console.log('PROVIDER GOT', JSON.stringify({ authorization: provider.received[0]?.headers.authorization, body: provider.received[0]?.body }));
  assert.equal(seen[0].authorization, `Bearer ${sentinel}`);
  assert.equal(seen[0].body, `{"t":"${sentinel}"}`);
  assert.equal(provider.received[0].headers.authorization, `Bearer ${REAL_ACCESS}`);
  assert.equal(provider.received[0].body, `{"t":"${REAL_ACCESS}"}`);
});

test('mutation: with an allow-all filter the foreign key and the real token in a body both land', { skip }, async () => {
  provider.clear();
  launcher.swapFilter(allowAll);
  try {
    const foreign = await request('/v1/messages', { headers: [`x-api-key: ${FOREIGN}`], data: '{"exfil":"data"}' });
    assert.equal(status(foreign), '200');
    assert.equal(provider.received[0].headers['x-api-key'], FOREIGN);
    const laundered = await request('/v1/messages', { headers: [`x-api-key: ${FOREIGN}`], data: '{"c":"$S11_FAKE_OAUTH_TOKEN"}' });
    assert.equal(status(laundered), '200');
    assert.equal(provider.received[1].body, `{"c":"${REAL_ACCESS}"}`, 'the real token left the sandbox under a foreign key');
  } finally {
    launcher.swapFilter(makeFilter({ providerHosts: ['localhost'], placeholders: () => [sentinel] }));
  }
});

test('control after the mutation: the real filter is back and refuses the foreign key again', { skip }, async () => {
  provider.clear();
  const r = await request('/v1/messages', { headers: [`x-api-key: ${FOREIGN}`] });
  assert.equal(status(r), '403');
  assert.equal(provider.received.length, 0);
});

test('row: a filter that throws fails closed', { skip }, async () => {
  provider.clear();
  launcher.swapFilter(async () => {
    throw new Error('boom');
  });
  try {
    const r = await request('/v1/messages', { headers: ['Authorization: Bearer $S11_FAKE_OAUTH_TOKEN'] });
    assert.equal(status(r), '403', r.stdout + r.stderr);
    assert.match(r.stdout, /policy check failed/);
    assert.equal(provider.received.length, 0);
  } finally {
    launcher.swapFilter(makeFilter({ providerHosts: ['localhost'], placeholders: () => [sentinel] }));
  }
});

test('row: overwrite mode replaces whatever credential the agent sent', { skip }, async () => {
  provider.clear();
  launcher.swapFilter(
    makeFilter({
      providerHosts: ['localhost'],
      placeholders: () => [sentinel],
      mode: 'overwrite',
      overwrite: { headers: [['Authorization', 'Bearer FAKE-LAUNCHER-OWN-TOKEN']] },
    }),
  );
  try {
    const r = await request('/v1/messages', { headers: [`x-api-key: ${FOREIGN}`, `X_Api_Key: ${FOREIGN}`, `Authorization: Bearer ${FOREIGN}`], data: '{"a":1}' });
    assert.equal(status(r), '200', r.stdout + r.stderr);
    const got = provider.received[0];
    assert.equal(got.headers.authorization, 'Bearer FAKE-LAUNCHER-OWN-TOKEN');
    assert.ok(!got.rawHeaders.some((h) => h.includes(FOREIGN)), 'no spelling of the foreign key arrived');
  } finally {
    launcher.swapFilter(makeFilter({ providerHosts: ['localhost'], placeholders: () => [sentinel] }));
  }
});

test('refresh: a refresh request with the file placeholder is refused by the real filter', { skip }, async () => {
  provider.clear();
  const cred = JSON.parse((await launcher.run(`cat '${credFile}'`)).stdout);
  const refreshSentinel = cred.claudeAiOauth.refreshToken;
  const r = await request('/oauth/token', { headers: ['content-type: application/json'], data: `{"grant_type":"refresh_token","refresh_token":"${refreshSentinel}"}` });
  assert.equal(status(r), '403', r.stdout + r.stderr);
  assert.equal(provider.received.length, 0);
});

test('refresh (mutation): allowed through, the real refresh token goes out and a real access token comes back into the sandbox', { skip }, async () => {
  provider.clear();
  const cred = JSON.parse((await launcher.run(`cat '${credFile}'`)).stdout);
  const refreshSentinel = cred.claudeAiOauth.refreshToken;
  launcher.swapFilter(allowAll);
  try {
    const r = await request('/oauth/token', { headers: ['content-type: application/json'], data: `{"grant_type":"refresh_token","refresh_token":"${refreshSentinel}"}` });
    assert.equal(status(r), '200', r.stdout + r.stderr);
    assert.equal(JSON.parse(provider.received[0].body).refresh_token, REAL_REFRESH);
    assert.match(r.stdout, /FAKE-NEW-REAL-ACCESS-TOKEN/, 'the new real token is readable by the agent: responses are not masked');
  } finally {
    launcher.swapFilter(makeFilter({ providerHosts: ['localhost'], placeholders: () => [sentinel] }));
  }
});

test('row: plain http to the provider host is refused', { skip }, async () => {
  provider.clear();
  const r = await launcher.run(`curl -sS -m 10 --noproxy '' -w '\\nHTTP=%{http_code}' -H 'Authorization: Bearer ${sentinel}' 'http://localhost:${provider.port}/v1/messages'`);
  console.log('PLAIN HTTP', JSON.stringify(r));
  assert.equal(status(r), '403', r.stdout + r.stderr);
  assert.equal(provider.received.length, 0);
});

test('host check: the real credential file on the host was not changed', { skip }, () => {
  assert.equal(JSON.parse(readFileSync(credFile, 'utf8')).claudeAiOauth.accessToken, REAL_ACCESS);
});
