DROP TRIGGER IF EXISTS trigger_update_workflow_cost ON activity_costs;
DROP FUNCTION IF EXISTS update_workflow_cost();
DROP FUNCTION IF EXISTS get_activity_cost(UUID, TEXT);
DROP FUNCTION IF EXISTS get_workflow_cost(UUID);
DROP MATERIALIZED VIEW IF EXISTS workflow_cost_summary;
ALTER TABLE workflows DROP COLUMN IF EXISTS budget_limit_usd;
ALTER TABLE workflows DROP COLUMN IF EXISTS total_cost_usd;
DROP TABLE IF EXISTS activity_costs;
