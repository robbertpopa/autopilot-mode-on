// Spike S11. The launcher: starts srt's SandboxManager with the request filter
// and runs one command inside it. Takes a config object, never reads a real
// sign-in; the credential it masks is whatever the caller names, and the spike
// only ever names fake ones.
import { spawn } from 'node:child_process';
import { SandboxManager } from '@anthropic-ai/sandbox-runtime';
import { makeFilter } from './filter.mjs';

// `swap` lets a mutation check replace the filter without a second initialize
// (SandboxManager is a per-process singleton).
export async function startLauncher({
  providerHosts,
  allowedDomains = providerHosts,
  filterOptions = {},
  credentials,
  filesystem,
  extraCaCertPaths = [],
  refuseOpaqueTunnels = true,
  excludeDomains,
}) {
  const inner = makeFilter({
    providerHosts,
    // Only the sentinel text is read from the registry, never the real value.
    placeholders: () => [...SandboxManager.getSentinelRegistry().entries()].map(([sentinel]) => sentinel),
    ...filterOptions,
  });
  const state = { filter: inner, seen: [] };
  const filterRequest = async (request, info) => {
    const decision = await state.filter(request, info);
    state.seen.push({ host: info?.target?.host, action: decision.action, reason: decision.reason });
    return decision;
  };

  await SandboxManager.initialize({
    network: {
      allowedDomains,
      deniedDomains: [],
      filterRequest,
      // A tunnel the filter cannot look inside would bypass it.
      refuseOpaqueTunnels,
      requireHostMatch: true,
      tlsTerminate: { extraCaCertPaths, excludeDomains },
    },
    filesystem,
    credentials,
  });

  return {
    state,
    swapFilter(fn) {
      state.filter = fn;
    },
    async run(command, { env = {}, timeoutMs = 20000 } = {}) {
      const wrapped = await SandboxManager.wrapWithSandbox(command);
      return await new Promise((resolve) => {
        const child = spawn('bash', ['-c', wrapped], { env: { PATH: process.env.PATH, HOME: process.env.HOME, ...env } });
        let stdout = '';
        let stderr = '';
        child.stdout.on('data', (d) => (stdout += d));
        child.stderr.on('data', (d) => (stderr += d));
        const timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs);
        child.on('close', (code) => {
          clearTimeout(timer);
          resolve({ code, stdout, stderr });
        });
      });
    },
    async stop() {
      await SandboxManager.reset();
    },
  };
}
