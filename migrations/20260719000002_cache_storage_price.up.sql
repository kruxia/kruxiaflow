-- Context-cache STORAGE price per million token-hours (e.g., Google Gemini
-- explicit caching bills storage for as long as the cache exists). NULL means
-- the provider has no storage charge (or the price is unknown); reported
-- token-hours are then recorded at cost 0 with a warning — there is no
-- sensible fallback price for a time-based dimension.
ALTER TABLE llm_models
ADD COLUMN cache_storage_price_per_million_token_hours NUMERIC;
