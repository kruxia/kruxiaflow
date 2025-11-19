-- LLM Providers (OpenAI, Anthropic, Google, Ollama, etc.)
CREATE TABLE llm_providers (
    name TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    api_endpoint TEXT,
    supports_completion BOOLEAN NOT NULL DEFAULT true,
    supports_embeddings BOOLEAN NOT NULL DEFAULT false,
    supports_streaming BOOLEAN NOT NULL DEFAULT false,
    requires_api_key BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- LLM Models with pricing information
CREATE TABLE llm_models (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    provider TEXT NOT NULL REFERENCES llm_providers(name) ON DELETE CASCADE,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    input_price_per_million NUMERIC NOT NULL DEFAULT 0,
    output_price_per_million NUMERIC NOT NULL DEFAULT 0,
    cached_input_price_per_million NUMERIC,
    supports_completion BOOLEAN NOT NULL DEFAULT true,
    supports_embeddings BOOLEAN NOT NULL DEFAULT false,
    context_window INTEGER,
    max_output_tokens INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider, name)
);
