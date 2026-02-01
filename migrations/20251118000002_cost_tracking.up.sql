-- Activity cost tracking table
CREATE TABLE activity_costs (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    workflow_id UUID NOT NULL,
    activity_key TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1,

    -- Cost details
    cost_usd DECIMAL NOT NULL,
    estimated_cost_usd DECIMAL,

    -- Token usage
    prompt_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    cached_tokens INTEGER,

    -- Provider details
    provider TEXT NOT NULL,
    model TEXT NOT NULL,

    -- Budget tracking
    activity_budget_limit_usd DECIMAL,
    workflow_budget_limit_usd DECIMAL,
    budget_exceeded BOOLEAN DEFAULT FALSE,
    budget_action TEXT, -- 'abort' or 'alert'

    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE
);

-- Index for activity cost queries
CREATE INDEX idx_activity_costs_activity
    ON activity_costs(workflow_id, activity_key);

-- Index for cost dashboard queries (BRIN for time-series efficiency)
CREATE INDEX idx_activity_costs_created
    ON activity_costs USING BRIN (created_at);

-- Index for provider analytics
CREATE INDEX idx_activity_costs_provider
    ON activity_costs(provider, model);

-- Add cost tracking columns to workflows table
ALTER TABLE workflows
    ADD COLUMN total_cost_usd DECIMAL DEFAULT 0.0,
    ADD COLUMN budget_limit_usd DECIMAL;

-- Function to get current workflow cost
CREATE OR REPLACE FUNCTION get_workflow_cost(p_workflow_id UUID)
RETURNS DECIMAL AS $$
    SELECT COALESCE(SUM(cost_usd), 0.0)
    FROM activity_costs
    WHERE workflow_id = p_workflow_id;
$$ LANGUAGE SQL STABLE;

-- Function to get current activity cost (across all attempts)
CREATE OR REPLACE FUNCTION get_activity_cost(p_workflow_id UUID, p_activity_key TEXT)
RETURNS DECIMAL AS $$
    SELECT COALESCE(SUM(cost_usd), 0.0)
    FROM activity_costs
    WHERE workflow_id = p_workflow_id
      AND activity_key = p_activity_key;
$$ LANGUAGE SQL STABLE;

-- Trigger to update workflow total_cost_usd on activity cost insert
CREATE OR REPLACE FUNCTION update_workflow_cost()
RETURNS TRIGGER AS $$
BEGIN
    UPDATE workflows
    SET total_cost_usd = get_workflow_cost(NEW.workflow_id)
    WHERE id = NEW.workflow_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_update_workflow_cost
AFTER INSERT ON activity_costs
FOR EACH ROW
EXECUTE FUNCTION update_workflow_cost();

-- View for cost dashboards
CREATE VIEW workflow_cost_summary AS
SELECT
    w.id AS workflow_id,
    w.definition_name AS workflow_name,
    w.total_cost_usd,
    w.budget_limit_usd,
    w.status,
    COUNT(ac.id) AS total_activities,
    COALESCE(SUM(ac.cost_usd), 0.0) AS actual_total_cost,
    MAX(ac.created_at) AS last_cost_update
FROM workflows w
LEFT JOIN activity_costs ac ON w.id = ac.workflow_id
GROUP BY w.id, w.definition_name, w.total_cost_usd, w.budget_limit_usd, w.status;
