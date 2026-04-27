// Payment flow stress: create + send invoice, then attempt /pay with an
// idempotency key, looking for cache hits on retries.
//
// Note: when STRIPE_SECRET_KEY isn't set, /pay returns 400 with "stripe is
// not configured". The script accepts that path (treats 400 as a non-error)
// so this scenario is still useful for measuring HTTP path latency without
// requiring real Stripe keys. Set Stripe env vars in docker-compose.yml to
// exercise the real Stripe call path.
//
// Run:  k6 run k6/payment-stress.js

import http from "k6/http";
import { check, sleep, group } from "k6";
import { Rate, Trend, Counter } from "k6/metrics";

import { BASE_URL, json, tenant, setupOrgWithClient } from "./lib/setup.js";

const errorRate = new Rate("errors");
const paymentDuration = new Trend("payment_duration", true);
const idempotentReplays = new Counter("idempotent_replays");

export const options = {
  scenarios: {
    payment_stress: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "20s", target: 10 },
        { duration: "1m", target: 50 },
        { duration: "1m", target: 50 },
        { duration: "20s", target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<2000", "p(99)<3000"],
    errors: ["rate<0.10"],
    payment_duration: ["p(95)<1500"],
  },
};

let ctx = null;

function createAndSendInvoice(h) {
  const dueDate = new Date(Date.now() + 30 * 86_400_000).toISOString();
  const create = http.post(
    `${BASE_URL}/v1/invoices`,
    json({
      client_id: ctx.clientId,
      invoice_number: `INV-PAY-${__VU}-${__ITER}-${Date.now()}`,
      line_items: [{ description: "Service", quantity: "1", unit_price: "100.00" }],
      tax_rate: "0",
      currency: "USD",
      due_date: dueDate,
    }),
    { headers: h },
  );
  if (create.status !== 201) return null;
  const invoice = JSON.parse(create.body);

  const send = http.post(`${BASE_URL}/v1/invoices/${invoice.id}/send`, null, {
    headers: h,
  });
  if (send.status !== 200) return null;
  return invoice.id;
}

export default function () {
  if (!ctx) {
    ctx = setupOrgWithClient("pay");
    if (!ctx) {
      errorRate.add(1);
      return;
    }
  }
  const h = tenant(ctx.accessToken, ctx.orgId);

  const invoiceId = createAndSendInvoice(h);
  if (!invoiceId) {
    errorRate.add(1);
    return;
  }

  group("pay with idempotency key", () => {
    const idemKey = `pay-${__VU}-${__ITER}-${Date.now()}`;
    const payHeaders = { ...h, "Idempotency-Key": idemKey };

    const t0 = Date.now();
    const first = http.post(
      `${BASE_URL}/v1/invoices/${invoiceId}/pay`,
      null,
      { headers: payHeaders },
    );
    paymentDuration.add(Date.now() - t0);

    // 201 = stripe configured + session created
    // 400 = stripe not configured (dev default) — still a "successful" rejection
    const ok = check(first, {
      "pay accepted-or-rejected-cleanly": (r) =>
        r.status === 201 || r.status === 400,
    });
    errorRate.add(!ok);

    // Replay with same key — should hit the idempotency cache and return
    // the same payload without creating a second Stripe session. With Stripe
    // unconfigured, the cache stores the rejection too, so this still works.
    if (first.status === 201) {
      const replay = http.post(
        `${BASE_URL}/v1/invoices/${invoiceId}/pay`,
        null,
        { headers: payHeaders },
      );
      if (replay.status === 200 || replay.status === 201) {
        const a = JSON.parse(first.body);
        const b = JSON.parse(replay.body);
        if (a.payment_id && a.payment_id === b.payment_id) {
          idempotentReplays.add(1);
        }
      }
    }
  });

  sleep(1);
}
