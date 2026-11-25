// Spike S11. A request filter for srt's `network.filterRequest` that keeps a
// masked subscription token from being used to send data out through the
// provider's own host. Not product code: nothing in the engine or an adapter
// loads it.
//
// What it must prevent: the agent holds only a placeholder, and srt swaps the
// placeholder for the real token on egress. Without a filter the agent can
// (a) send a request to the provider under a key of its own, and (b) put the
// placeholder where srt also substitutes it (a body, a query, another header)
// so the real token lands in a place the agent can read back.

export const DEFAULT_CREDENTIAL_HEADERS = [
  'authorization',
  'proxy-authorization',
  'x-api-key',
  'api-key',
  'cookie',
];

// Same folding srt uses for removeHeaders, so one spelling cannot slip past:
// `X_Api_Key` and `x.api.key` reach some upstreams as `x-api-key`.
export function fold(name) {
  return name.toLowerCase().replace(/[_.]/g, '-');
}

function deny(reason, status = 403) {
  return { action: 'deny', reason, status, mark: 's11-filter' };
}

// `Bearer <placeholder>` or the bare placeholder, nothing else. The value is
// compared whole, so trailing text or a second token is a different credential.
function carriesPlaceholder(value, placeholder) {
  if (value === placeholder) return true;
  const m = /^Bearer ([^\s]+)$/i.exec(value);
  return m !== null && m[1] === placeholder;
}

// Streams the body once and reports whether any needle occurs. Bounded: a body
// over `limit` bytes is refused rather than scanned part way, because an
// unscanned tail could hold the placeholder.
export async function bodyHolds(request, needles, limit) {
  if (request.body === null) return { found: false, tooLarge: false };
  const reader = request.body.getReader();
  const wants = needles.map((n) => Buffer.from(n));
  const keep = Math.max(...wants.map((w) => w.length)) - 1;
  let carry = Buffer.alloc(0);
  let seen = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return { found: false, tooLarge: false };
    seen += value.length;
    if (seen > limit) {
      await reader.cancel();
      return { found: false, tooLarge: true };
    }
    const window = Buffer.concat([carry, Buffer.from(value)]);
    if (wants.some((w) => window.includes(w))) {
      await reader.cancel();
      return { found: true, tooLarge: false };
    }
    carry = window.subarray(Math.max(0, window.length - keep));
  }
}

// `mode: 'reject'`: deny unless the request carries exactly the placeholder.
// `mode: 'overwrite'`: allow, but drop every credential header the agent sent
// and set the launcher's own. No comparison to get wrong; the placeholder never
// reaches the provider and srt's header substitution has nothing to do.
export function makeFilter({
  providerHosts,
  placeholders,
  mode = 'reject',
  credentialHeaders = DEFAULT_CREDENTIAL_HEADERS,
  overwrite,
  bodyLimit = 32 * 1024 * 1024,
}) {
  const hosts = new Set(providerHosts.map((h) => h.toLowerCase()));
  const credNames = new Set(credentialHeaders.map(fold));
  // srt mints the placeholders when it wraps a command, after the filter is
  // built, so they may be given as a function that is read on every request.
  const marksNow = () => {
    const marks = typeof placeholders === 'function' ? [...placeholders()] : [...placeholders];
    // A short placeholder would match unrelated text, and none at all would
    // make every request look unauthenticated or, worse, trivially clean.
    if (marks.length === 0 || marks.some((p) => typeof p !== 'string' || p.length < 8)) return undefined;
    return marks;
  };
  if (typeof placeholders !== 'function' && marksNow() === undefined) {
    throw new Error('placeholders must be at least 8 characters each');
  }
  if (mode === 'overwrite' && !overwrite) throw new Error('overwrite mode needs overwrite.headers');

  return async function filterRequest(request, info) {
    // Everything below is judged on what the proxy parsed, not on what the
    // client claims. A missing `info` means a proxy version this was not
    // written for, so it fails closed.
    const marks = marksNow();
    if (!marks) return deny('filter: no placeholder is registered');
    if (!info || !info.target || !info.rawHeaders) return deny('filter: no request info');
    if (info.scheme !== 'https') return deny('filter: provider requests must be https');
    const host = info.target.host.toLowerCase();
    if (!hosts.has(host)) return deny(`filter: ${host} is not a provider host`);
    if (info.requestTargetIsAmbiguous) return deny('filter: ambiguous request target');
    if (info.unshownBody && (await info.unshownBody())) {
      return deny('filter: body on a method whose body the filter cannot read');
    }

    const raw = info.rawHeaders;
    const credentials = [];
    for (let i = 0; i + 1 < raw.length; i += 2) {
      if (credNames.has(fold(raw[i]))) credentials.push([raw[i], raw[i + 1]]);
    }

    if (mode === 'overwrite') {
      // Any placeholder the agent put anywhere is still refused: with the
      // credential headers replaced it is harmless in a header, but in a body
      // srt would turn it into the real token and the model could echo it.
      const hit = await placeholderOutsideCredentialHeaders(request, info, raw, credNames, marks, bodyLimit, false);
      if (hit) return deny(hit);
      return {
        action: 'allow',
        removeHeaders: [...credNames],
        setHeaders: overwrite.headers,
      };
    }

    if (credentials.length !== 1) {
      return deny(
        credentials.length === 0
          ? 'filter: no credential header; provider requests must carry the placeholder'
          : 'filter: more than one credential header',
      );
    }
    const [, value] = credentials[0];
    if (!marks.some((p) => carriesPlaceholder(value, p))) {
      return deny('filter: credential is not the placeholder');
    }
    const hit = await placeholderOutsideCredentialHeaders(request, info, raw, credNames, marks, bodyLimit, true);
    if (hit) return deny(hit);
    return { action: 'allow' };
  };
}

async function placeholderOutsideCredentialHeaders(request, info, raw, credNames, marks, bodyLimit, skipCredentialHeaders) {
  for (let i = 0; i + 1 < raw.length; i += 2) {
    if (skipCredentialHeaders && credNames.has(fold(raw[i]))) continue;
    if (marks.some((p) => raw[i + 1].includes(p) || raw[i].includes(p))) {
      return 'filter: placeholder outside the credential header';
    }
  }
  // srt does not substitute in the URL, but a placeholder there is still a
  // laundering attempt. Decode first so `%66ake_value_` is caught too.
  const target = info.requestTarget ?? '';
  let decoded = target;
  try {
    decoded = decodeURIComponent(target);
  } catch {
    return 'filter: undecodable request target';
  }
  if (marks.some((p) => target.includes(p) || decoded.includes(p))) {
    return 'filter: placeholder in the request target';
  }
  const { found, tooLarge } = await bodyHolds(request, marks, bodyLimit);
  if (tooLarge) return 'filter: body too large to scan';
  if (found) return 'filter: placeholder in the body';
  return undefined;
}
