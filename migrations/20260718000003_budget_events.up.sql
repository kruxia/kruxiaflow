-- Budget enforcement events as first-class cost line items (cost visibility, G5).
--
-- budget_event values:
--   'abort'     - the orchestrator's pre-execution check aborted the activity
--                 (zero-cost row; estimated_cost_usd holds the cheapest estimate
--                 that still exceeded the budget)
--   'downgrade' - the fallback chain skipped one or more models for budget
--                 reasons before a cheaper model succeeded (ordinary cost row)
--   NULL        - ordinary cost row, no enforcement fired
ALTER TABLE activity_costs ADD COLUMN budget_event TEXT;

-- Partial index: budget events are rare relative to cost rows; analytics lists
-- them in date ranges.
CREATE INDEX idx_activity_costs_budget_event
    ON activity_costs (created_at)
    WHERE budget_event IS NOT NULL;

-- Skip the workflows total-cost update for zero-cost rows. A zero-cost row
-- (budget abort marker) cannot change the total, and the orchestrator records
-- it from a separate connection while its event-processing transaction may
-- hold the workflows row lock — an unconditional trigger UPDATE deadlocks on
-- that lock.
DROP TRIGGER trigger_update_workflow_cost ON activity_costs;
CREATE TRIGGER trigger_update_workflow_cost
AFTER INSERT ON activity_costs
FOR EACH ROW
WHEN (NEW.cost_usd <> 0)
EXECUTE FUNCTION update_workflow_cost();
