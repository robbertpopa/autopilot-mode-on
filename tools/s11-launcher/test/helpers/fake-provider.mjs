// A fake provider host on loopback, with a throwaway CA and leaf made here. It
// records exactly what reaches it, which is what each probe row is judged on.
import https from 'node:https';
import tls from 'node:tls';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import forge from 'node-forge';

function pki() {
  const caKeys = forge.pki.rsa.generateKeyPair(2048);
  const ca = forge.pki.createCertificate();
  ca.publicKey = caKeys.publicKey;
  ca.serialNumber = '01';
  ca.validity.notBefore = new Date(Date.now() - 3600e3);
  ca.validity.notAfter = new Date(Date.now() + 86400e3);
  const caAttrs = [{ name: 'commonName', value: 'S11 fake provider CA' }];
  ca.setSubject(caAttrs);
  ca.setIssuer(caAttrs);
  ca.setExtensions([{ name: 'basicConstraints', cA: true }, { name: 'keyUsage', keyCertSign: true }]);
  ca.sign(caKeys.privateKey, forge.md.sha256.create());

  const keys = forge.pki.rsa.generateKeyPair(2048);
  const leaf = forge.pki.createCertificate();
  leaf.publicKey = keys.publicKey;
  leaf.serialNumber = '02';
  leaf.validity.notBefore = new Date(Date.now() - 3600e3);
  leaf.validity.notAfter = new Date(Date.now() + 86400e3);
  leaf.setSubject([{ name: 'commonName', value: 'localhost' }]);
  leaf.setIssuer(caAttrs);
  leaf.setExtensions([
    { name: 'subjectAltName', altNames: [{ type: 2, value: 'localhost' }, { type: 7, ip: '127.0.0.1' }] },
  ]);
  leaf.sign(caKeys.privateKey, forge.md.sha256.create());
  return {
    caPem: forge.pki.certificateToPem(ca),
    certPem: forge.pki.certificateToPem(leaf),
    keyPem: forge.pki.privateKeyToPem(keys.privateKey),
  };
}

export async function startFakeProvider() {
  const { caPem, certPem, keyPem } = pki();
  const dir = mkdtempSync(join(tmpdir(), 's11-provider-'));
  const caPath = join(dir, 'fake-provider-ca.pem');
  writeFileSync(caPath, caPem);
  // The proxy's upstream leg verifies certificates against the host's default
  // roots, so the fake CA is added to this process's roots (Node 22.19 or newer).
  tls.setDefaultCACertificates([...tls.getCACertificates('default'), caPem]);
  const received = [];
  const server = https.createServer({ cert: certPem, key: keyPem }, (req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const body = Buffer.concat(chunks).toString('utf8');
      received.push({ method: req.method, path: req.url, host: req.headers.host, headers: req.headers, rawHeaders: req.rawHeaders, body });
      res.setHeader('content-type', 'application/json');
      if (req.url.startsWith('/oauth/token')) {
        res.end(JSON.stringify({ access_token: 'FAKE-NEW-REAL-ACCESS-TOKEN', refresh_token: 'FAKE-NEW-REAL-REFRESH-TOKEN' }));
      } else {
        res.end(JSON.stringify({ ok: true }));
      }
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return {
    port: server.address().port,
    caPath,
    dir,
    received,
    clear: () => (received.length = 0),
    close: () => new Promise((r) => server.close(r)),
  };
}
