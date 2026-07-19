DROP TRIGGER trigger_update_workflow_cost ON activity_costs;
CREATE TRIGGER trigger_update_workflow_cost
AFTER INSERT ON activity_costs
FOR EACH ROW
EXECUTE FUNCTION update_workflow_cost();

DROP INDEX IF EXISTS idx_activity_costs_budget_event;
ALTER TABLE activity_costs DROP COLUMN IF EXISTS budget_event;
