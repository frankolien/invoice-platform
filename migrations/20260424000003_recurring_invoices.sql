CREATE TABLE recurring_invoices (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    client_id UUID NOT NULL REFERENCES clients(id) ON DELETE RESTRICT,
    line_items JSONB NOT NULL DEFAULT '[]'::jsonb,
    frequency TEXT NOT NULL CHECK (frequency IN ('weekly', 'monthly', 'quarterly', 'yearly')),
    tax_rate NUMERIC(6,4) NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'cancelled')),
    start_date TIMESTAMPTZ NOT NULL,
    end_date TIMESTAMPTZ,
    next_run_at TIMESTAMPTZ,
    last_run_at TIMESTAMPTZ,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes mirror the TS BullMQ scanner query shape:
--   WHERE status = 'active' AND next_run_at <= now()
CREATE INDEX idx_recurring_org_status ON recurring_invoices(org_id, status);
CREATE INDEX idx_recurring_due ON recurring_invoices(status, next_run_at)
    WHERE status = 'active' AND next_run_at IS NOT NULL;
