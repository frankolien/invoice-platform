// Auth endpoint load: register -> login -> refresh
//
// Run:  k6 run k6/auth-load.js
// Override target: k6 run --env BASE_URL=http://staging:3000 k6/auth-load.js

import http from "k6/http";
import { check, sleep } from "k6";
import { Rate, Trend } from "k6/metrics";

import { BASE_URL, json, headers } from "./lib/setup.js";

const errorRate = new Rate("errors");
const loginDuration = new Trend("login_duration", true);
const registerDuration = new Trend("register_duration", true);

export const options = {
  scenarios: {
    auth_load: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "30s", target: 50 },
        { duration: "1m", target: 100 },
        { duration: "30s", target: 100 },
        { duration: "30s", target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<500", "p(99)<1000"],
    errors: ["rate<0.05"],
    login_duration: ["p(95)<300"],
    register_duration: ["p(95)<500"],
  },
};

export default function () {
  const uniqueId = `${__VU}-${__ITER}-${Date.now()}`;
  const email = `loadtest-${uniqueId}@loadtest.local`;
  const password = "loadtest-password-123";

  // Register
  const registerRes = http.post(
    `${BASE_URL}/v1/auth/register`,
    json({ email, password, name: `User ${uniqueId}` }),
    { headers },
  );
  registerDuration.add(registerRes.timings.duration);
  const registered = check(registerRes, {
    "register 201": (r) => r.status === 201,
  });
  errorRate.add(!registered);
  if (!registered) return;

  // Login
  const loginRes = http.post(
    `${BASE_URL}/v1/auth/login`,
    json({ email, password }),
    { headers },
  );
  loginDuration.add(loginRes.timings.duration);
  const loggedIn = check(loginRes, {
    "login 200": (r) => r.status === 200,
    "has access_token": (r) => JSON.parse(r.body).access_token !== undefined,
  });
  errorRate.add(!loggedIn);
  if (!loggedIn) return;

  const { refresh_token } = JSON.parse(loginRes.body);

  // Refresh
  const refreshRes = http.post(
    `${BASE_URL}/v1/auth/refresh`,
    json({ refresh_token }),
    { headers },
  );
  check(refreshRes, { "refresh 200": (r) => r.status === 200 });

  sleep(0.5);
}
