// Mixed traffic: 60% reads / 30% writes / 10% payment attempts.
// Tests how the system behaves under realistic concurrent load.
//
// Run:  k6 run k6/mixed-scenario.js

import http from "k6/http";
import { check, sleep, group } from "k6";
import { Rate, Trend } from "k6/metrics";

import { BASE_URL, json, tenant, setupOrgWithClient } from "./lib/setup.js";

const errorRate = new Rate("errors");
const readDuration = new Trend("read_duration", true);
const writeDuration = new Trend("write_duration", true);
const paymentDuration = new Trend("payment_duration", true);

export const options = {
  scenarios: {
    readers: {
      executor: "constant-vus",
      vus: 60,
      duration: "3m",
      exec: "readScenario",
    },
    writers: {
      executor: "constant-vus",
      vus: 30,
      duration: "3m",
      exec: "writeScenario",
    },
    payers: {
      executor: "constant-vus",
      vus: 10,
      duration: "3m",
      exec: "paymentScenario",
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<1500", "p(99)<3000"],
    errors: ["rate<0.05"],
    read_duration: ["p(95)<500"],
    write_duration: ["p(95)<1000"],
    payment_duration: ["p(95)<2000"],
  },
};

// Per-VU auth context, lazily initialized. Each scenario keeps its own copy.
let readCtx = null;
let writeCtx = null;
let payCtx = null;

export function readScenario() {
  if (!readCtx) {
    readCtx = setupOrgWithClient("rd");
    if (!readCtx) {
      errorRate.add(1);
      return;
    }
    // Seed one invoice so list/get have something to find.
    const h = tenant(readCtx.accessToken, readCtx.orgId);
    http.post(
      `${BASE_URL}/v1/invoices`,
      json({
        client_id: readCtx.clientId,
        invoice_number: `INV-RD-SEED-${__VU}-${Date.now()}`,
        line_items: [{ description: "seed", quantity: "1", unit_price: "10" }],
        currency: "USD",
        due_date: new Date(Date.now() + 30 * 86_400_000).toISOString(),
      }),
      { headers: h },
    );
  }
  const h = tenant(readCtx.accessToken, readCtx.orgId);

  const t0 = Date.now();
  const list = http.get(`${BASE_URL}/v1/invoices?page_size=20`, { headers: h });
  readDuration.add(Date.now() - t0);
  const ok = check(list, { "list 200": (r) => r.status === 200 });
  errorRate.add(!ok);

  if (ok) {
    const arr = JSON.parse(list.body);
    if (arr.length) {
      const get = http.get(`${BASE_URL}/v1/invoices/${arr[0].id}`, { headers: h });
      check(get, { "get 200": (r) => r.status === 200 });
    }
  }

  sleep(0.5);
}

export function writeScenario() {
  if (!writeCtx) {
    writeCtx = setupOrgWithClient("wr");
    if (!writeCtx) {
      errorRate.add(1);
      return;
    }
  }
  const h = tenant(writeCtx.accessToken, writeCtx.orgId);
  const dueDate = new Date(Date.now() + 30 * 86_400_000).toISOString();

  const t0 = Date.now();
  const res = http.post(
    `${BASE_URL}/v1/invoices`,
    json({
      client_id: writeCtx.clientId,
      invoice_number: `INV-WR-${__VU}-${__ITER}-${Date.now()}`,
      line_items: [
        { description: "Item A", quantity: "1", unit_price: "75" },
        { description: "Item B", quantity: "3", unit_price: "25" },
      ],
      tax_rate: "0.08",
      currency: "USD",
      due_date: dueDate,
    }),
    { headers: h },
  );
  writeDuration.add(Date.now() - t0);
  const ok = check(res, { "create 201": (r) => r.status === 201 });
  errorRate.add(!ok);

  sleep(1);
}

export function paymentScenario() {
  if (!payCtx) {
    payCtx = setupOrgWithClient("pay");
    if (!payCtx) {
      errorRate.add(1);
      return;
    }
  }
  const h = tenant(payCtx.accessToken, payCtx.orgId);
  const dueDate = new Date(Date.now() + 30 * 86_400_000).toISOString();

  const create = http.post(
    `${BASE_URL}/v1/invoices`,
    json({
      client_id: payCtx.clientId,
      invoice_number: `INV-MIXPAY-${__VU}-${__ITER}-${Date.now()}`,
      line_items: [{ description: "Service", quantity: "1", unit_price: "100" }],
      currency: "USD",
      due_date: dueDate,
    }),
    { headers: h },
  );
  if (create.status !== 201) {
    errorRate.add(1);
    return;
  }
  const invoiceId = JSON.parse(create.body).id;

  http.post(`${BASE_URL}/v1/invoices/${invoiceId}/send`, null, { headers: h });

  const t0 = Date.now();
  const pay = http.post(
    `${BASE_URL}/v1/invoices/${invoiceId}/pay`,
    null,
    { headers: { ...h, "Idempotency-Key": `mix-${__VU}-${__ITER}-${Date.now()}` } },
  );
  paymentDuration.add(Date.now() - t0);
  // 201 (stripe configured) or 400 (stripe disabled) both fine.
  const ok = check(pay, {
    "pay accepted-or-rejected-cleanly": (r) => r.status === 201 || r.status === 400,
  });
  errorRate.add(!ok);

  sleep(1);
}
