-- Cache-write pricing for the LLM model catalog.
-- Anthropic bills prompt-cache writes at a premium over input tokens (1.25x
-- for the default 5-minute TTL). When present, reported cache_creation_tokens
-- are billed at this price; when NULL, they fall back to the input-token price.
ALTER TABLE llm_models
    ADD COLUMN cache_write_price_per_million NUMERIC;
