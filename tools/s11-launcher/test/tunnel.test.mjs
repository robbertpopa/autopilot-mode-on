// An opaque tunnel is a CONNECT the proxy cannot look inside, so the filter
// never sees it. refuseOpaqueTunnels must close that. The same request is run
// with the switch on (refused) and off (a foreign key lands), so the refusal is
// attributed to the switch and not to the host being unreachable.
import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeProvider } from './helpers/fake-provider.mjs';
import { startLauncher } from '../launcher.mjs';

const FOREIGN = 'sk-ant-attacker-key-0000000000000000';
const skip = process.platform !== 'linux' ? 'needs Linux or WSL2' : false;
let provider;

before(async () => {
  if (!skip) provider = await startFakeProvider();
});
after(async () => {
  await provider?.close();
});

async function run(refuseOpaqueTunnels) {
  const l = await startLauncher({
    providerHosts: ['localhost'],
    allowedDomains: ['localhost', '127.0.0.1'],
    // 127.0.0.1 is allowed by the domain list but not terminated: the opaque path.
    excludeDomains: ['127.0.0.1'],
    refuseOpaqueTunnels,
    extraCaCertPaths: [provider.caPath],
    filesystem: { denyRead: [], allowRead: [], allowWrite: [], denyWrite: [] },
  });
  try {
    provider.clear();
    return await l.run(`curl -sS -m 10 --noproxy '' -w '\nHTTP=%{http_code}' -H 'x-api-key: ${FOREIGN}' --data '{"exfil":1}' 'https://127.0.0.1:${provider.port}/v1/messages'`);
  } finally {
    await l.stop();
  }
}

test('row: an opaque tunnel to an allowed host is refused', { skip }, async () => {
  const r = await run(true);
  assert.doesNotMatch(r.stdout, /HTTP=200/, r.stdout + r.stderr);
  assert.equal(provider.received.length, 0);
});

test('mutation: with refuseOpaqueTunnels off the foreign key lands at the provider', { skip }, async () => {
  const r = await run(false);
  assert.match(r.stdout, /HTTP=200/, r.stdout + r.stderr);
  assert.equal(provider.received[0].headers['x-api-key'], FOREIGN);
});
