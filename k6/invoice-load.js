// Invoice CRUD load: per-VU auth setup, then create + list + get loop.
//
// Run:  k6 run k6/invoice-load.js

import http from "k6/http";
import { check, sleep, group } from "k6";
import { Rate, Trend } from "k6/metrics";

import { BASE_URL, json, tenant, setupOrgWithClient } from "./lib/setup.js";

const errorRate = new Rate("errors");
const createInvoiceDuration = new Trend("create_invoice_duration", true);
const listInvoiceDuration = new Trend("list_invoice_duration", true);
const getInvoiceDuration = new Trend("get_invoice_duration", true);

export const options = {
  scenarios: {
    invoice_load: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "30s", target: 50 },
        { duration: "1m", target: 200 },
        { duration: "1m", target: 200 },
        { duration: "30s", target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<1000", "p(99)<2000"],
    errors: ["rate<0.05"],
    create_invoice_duration: ["p(95)<800"],
    list_invoice_duration: ["p(95)<500"],
    get_invoice_duration: ["p(95)<300"],
  },
};

// One auth context per VU, set up lazily on first iteration. k6 doesn't share
// state across VUs (each is its own JS runtime), so this is just a per-VU
// memo.
let ctx = null;

export default function () {
  if (!ctx) {
    ctx = setupOrgWithClient("inv");
    if (!ctx) {
      errorRate.add(1);
      return;
    }
  }
  const h = tenant(ctx.accessToken, ctx.orgId);
  const dueDate = new Date(Date.now() + 30 * 86_400_000).toISOString();

  group("create invoice", () => {
    // Our API requires a unique invoice_number per org; embed VU + ITER to avoid collisions.
    const res = http.post(
      `${BASE_URL}/v1/invoices`,
      json({
        client_id: ctx.clientId,
        invoice_number: `INV-${__VU}-${__ITER}-${Date.now()}`,
        line_items: [
          { description: `Service ${__ITER}`, quantity: "1", unit_price: "100.00" },
          { description: "Support", quantity: "2", unit_price: "50.00" },
        ],
        // tax_rate is a Decimal (rate, not percent). 0.10 = 10%.
        tax_rate: "0.10",
        currency: "USD",
        due_date: dueDate,
      }),
      { headers: h },
    );
    createInvoiceDuration.add(res.timings.duration);
    const ok = check(res, { "create invoice 201": (r) => r.status === 201 });
    errorRate.add(!ok);
  });

  group("list invoices", () => {
    const res = http.get(`${BASE_URL}/v1/invoices?page_size=20`, { headers: h });
    listInvoiceDuration.add(res.timings.duration);
    const ok = check(res, {
      "list invoices 200": (r) => r.status === 200,
      "is array": (r) => Array.isArray(JSON.parse(r.body)),
    });
    errorRate.add(!ok);

    // Walk a couple of pages so we exercise pagination.
    if (ok) {
      const page2 = http.get(`${BASE_URL}/v1/invoices?page=2&page_size=20`, {
        headers: h,
      });
      check(page2, { "page 2 200": (r) => r.status === 200 });
    }
  });

  group("get single invoice", () => {
    const listRes = http.get(`${BASE_URL}/v1/invoices?page_size=1`, { headers: h });
    if (listRes.status !== 200) return;
    const arr = JSON.parse(listRes.body);
    if (!arr.length) return;
    const res = http.get(`${BASE_URL}/v1/invoices/${arr[0].id}`, { headers: h });
    getInvoiceDuration.add(res.timings.duration);
    const ok = check(res, { "get invoice 200": (r) => r.status === 200 });
    errorRate.add(!ok);
  });

  sleep(0.5);
}
