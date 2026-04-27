// Shared setup helpers for k6 scripts targeting the Rust API.
//
// Why these exist: every load scenario needs an authenticated VU bound to an
// org (and often a client), and the boilerplate is identical across scripts.

import http from "k6/http";

const BASE_URL = __ENV.BASE_URL || "http://localhost:3000";

const json = (body) => JSON.stringify(body);
const headers = { "Content-Type": "application/json" };
const auth = (token) => ({ ...headers, Authorization: `Bearer ${token}` });
const tenant = (token, orgId) => ({ ...auth(token), "x-org-id": orgId });

// Returns { accessToken } or null if registration failed.
export function registerAndLogin(prefix) {
  const email = `${prefix}-${__VU}-${__ITER}-${Date.now()}@loadtest.local`;
  const password = "loadtest-password-123";

  const reg = http.post(
    `${BASE_URL}/v1/auth/register`,
    json({ email, password, name: `${prefix} ${__VU}` }),
    { headers },
  );
  if (reg.status !== 201) return null;

  const body = JSON.parse(reg.body);
  return { email, password, accessToken: body.access_token };
}

// Returns { accessToken, orgId } or null on any failure.
export function setupOrg(prefix) {
  const u = registerAndLogin(prefix);
  if (!u) return null;

  const slug = `${prefix}-${__VU}-${Date.now()}`.slice(0, 60);
  const orgRes = http.post(
    `${BASE_URL}/v1/organizations`,
    json({ name: `${prefix} Org`, slug }),
    { headers: auth(u.accessToken) },
  );
  if (orgRes.status !== 201) return null;
  const orgId = JSON.parse(orgRes.body).id;
  return { accessToken: u.accessToken, orgId };
}

// Returns { accessToken, orgId, clientId } or null on any failure.
export function setupOrgWithClient(prefix) {
  const ctx = setupOrg(prefix);
  if (!ctx) return null;

  const clientRes = http.post(
    `${BASE_URL}/v1/clients`,
    json({
      name: `${prefix} client`,
      email: `client-${__VU}-${Date.now()}@loadtest.local`,
    }),
    { headers: tenant(ctx.accessToken, ctx.orgId) },
  );
  if (clientRes.status !== 201) return null;
  const clientId = JSON.parse(clientRes.body).id;
  return { ...ctx, clientId };
}

export { BASE_URL, json, headers, auth, tenant };
