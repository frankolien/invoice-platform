CREATE TABLE webhook_subscriptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    events TEXT[] NOT NULL,
    secret TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'inactive')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_webhook_subscriptions_org ON webhook_subscriptions(org_id);
-- GIN index on events so dispatch can fan out efficiently:
-- WHERE org_id = $1 AND status = 'active' AND $2 = ANY(events)
CREATE INDEX idx_webhook_subscriptions_events ON webhook_subscriptions USING GIN (events);
