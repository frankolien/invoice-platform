CREATE TABLE payments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    invoice_id UUID NOT NULL REFERENCES invoices(id) ON DELETE RESTRICT,
    stripe_checkout_session_id TEXT,
    stripe_payment_intent_id TEXT,
    amount NUMERIC(14,2) NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN (
        'pending', 'succeeded', 'failed', 'refunded', 'partially_refunded'
    )),
    method TEXT,
    failure_reason TEXT,
    paid_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_payments_org_invoice ON payments(org_id, invoice_id);
CREATE INDEX idx_payments_org_status_created ON payments(org_id, status, created_at DESC);
CREATE INDEX idx_payments_stripe_payment_intent ON payments(stripe_payment_intent_id)
    WHERE stripe_payment_intent_id IS NOT NULL;
CREATE INDEX idx_payments_stripe_checkout_session ON payments(stripe_checkout_session_id)
    WHERE stripe_checkout_session_id IS NOT NULL;

-- Idempotency keys (Redis is primary, this is a DB fallback / audit trail)
-- For webhook deduplication we use Redis (48h TTL) which is more than enough.
-- Only adding the payments table here; webhook dedup lives in Redis.
