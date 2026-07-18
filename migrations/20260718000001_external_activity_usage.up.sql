-- External activity usage reporting (2026-07-18 spec)
--
-- 1. Relax activity_costs.provider/.model to nullable: lump-sum cost rows
--    (external activities reporting only a total cost_usd) and non-LLM cost
--    line items have no provider/model.
ALTER TABLE activity_costs
    ALTER COLUMN provider DROP NOT NULL,
    ALTER COLUMN model DROP NOT NULL;

-- 2. Store workflow-level settings (budget etc.) with the definition.
--    Previously the top-level `settings:` block in workflow YAML was parsed
--    and silently dropped; workflow-level budgets require it to be persisted
--    so submission can copy settings.budget.limit into workflows.budget_limit_usd.
ALTER TABLE workflow_definitions
    ADD COLUMN settings JSONB;
