-- Schema for the kruxiaflow_examples database.
-- Creates all tables referenced by example workflows.
--
-- This script runs automatically on first postgres startup via
-- docker-entrypoint-initdb.d when using docker-compose.override.yml.

\connect kruxiaflow_examples;

-- Example 02: User Validation
CREATE TABLE valid_users (
    id         SERIAL PRIMARY KEY,
    email      TEXT NOT NULL UNIQUE,
    validated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE invalid_users (
    id         SERIAL PRIMARY KEY,
    email      TEXT NOT NULL UNIQUE,
    reason     TEXT,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Example 04: Content Moderation
CREATE TABLE moderation_log (
    id          SERIAL PRIMARY KEY,
    content_id  TEXT NOT NULL,
    decision    TEXT NOT NULL,
    cost        DECIMAL(10, 6),
    tokens      INTEGER,
    moderated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Examples 05, 05a, 05b, 05c: Research Assistant
CREATE TABLE research_log (
    id            SERIAL PRIMARY KEY,
    question      TEXT NOT NULL,
    answer        TEXT NOT NULL,
    provider      TEXT NOT NULL,
    model         TEXT NOT NULL,
    cost          DECIMAL(10, 6),
    prompt_tokens  INTEGER,
    output_tokens  INTEGER,
    total_tokens   INTEGER,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Example 06a: FAQ Bot with Semantic Caching
CREATE TABLE faq_log (
    id         SERIAL PRIMARY KEY,
    question   TEXT NOT NULL,
    answer     TEXT NOT NULL,
    cost       DECIMAL(10, 6),
    cache_hit  BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Examples 06b, 06c: RAG Index Builder and RAG Query
-- Requires pgvector extension
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE document_chunks (
    id         SERIAL PRIMARY KEY,
    content    TEXT NOT NULL,
    embedding  vector(3072) NOT NULL,
    metadata   JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Note: IVFFlat and HNSW indexes are limited to 2000 dimensions; vector(3072)
-- (OpenAI text-embedding-3-large) exceeds that limit. No index is needed for
-- the examples database — sequential scan is fine for small demo datasets.

-- Example 06c: RAG Query Q&A Log
CREATE TABLE qa_log (
    id             SERIAL PRIMARY KEY,
    question       TEXT NOT NULL,
    answer         TEXT NOT NULL,
    chunks_used    INTEGER NOT NULL,
    embedding_cost DECIMAL(10, 6),
    llm_cost       DECIMAL(10, 6),
    total_cost     DECIMAL(10, 6),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Example 10: Order Processing
CREATE TABLE payments (
    id          SERIAL PRIMARY KEY,
    customer_id TEXT NOT NULL,
    amount      DECIMAL(10, 2) NOT NULL,
    currency    TEXT NOT NULL DEFAULT 'USD',
    status      TEXT NOT NULL DEFAULT 'completed',
    product_id  TEXT,
    quantity    INT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE orders (
    id              SERIAL PRIMARY KEY,
    customer_id     TEXT NOT NULL,
    customer_email  TEXT NOT NULL,
    product_id      TEXT NOT NULL,
    quantity        INT NOT NULL,
    amount          DECIMAL(10, 2) NOT NULL,
    payment_txn_id  TEXT,
    reservation_id  TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE inventory (
    product_id TEXT PRIMARY KEY,
    available  INT NOT NULL DEFAULT 0,
    reserved   INT NOT NULL DEFAULT 0
);

-- Seed some inventory for testing example 10
INSERT INTO inventory (product_id, available, reserved) VALUES
    ('prod_001', 100, 0),
    ('prod_002', 50, 0),
    ('prod_003', 25, 0);
