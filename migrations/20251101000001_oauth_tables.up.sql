-- Create OAuth authentication tables
-- Used by PostgresAuthService for JWT token issuance and validation

-- Users table (for password grant flow and user management)
CREATE TABLE oauth_users (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    username TEXT UNIQUE NOT NULL,
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,  -- bcrypt hash
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_users_username ON oauth_users(username);
CREATE INDEX idx_oauth_users_email ON oauth_users(email);

-- Service accounts/clients (for client_credentials flow)
CREATE TABLE oauth_clients (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    client_id TEXT UNIQUE NOT NULL,
    client_secret_hash TEXT NOT NULL,  -- bcrypt hash
    name TEXT NOT NULL,
    description TEXT,
    scopes TEXT[] NOT NULL DEFAULT '{}',
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_oauth_clients_client_id ON oauth_clients(client_id);

-- Refresh tokens (for token refresh)
CREATE TABLE oauth_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    token_hash TEXT UNIQUE NOT NULL,  -- SHA-256 hash for efficient indexed lookup
    user_id UUID REFERENCES oauth_users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_oauth_refresh_tokens_user ON oauth_refresh_tokens(user_id);
CREATE INDEX idx_oauth_refresh_tokens_expires ON oauth_refresh_tokens USING BRIN (expires_at);

-- Trigger to update updated_at column
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_oauth_users_updated_at BEFORE UPDATE ON oauth_users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_oauth_clients_updated_at BEFORE UPDATE ON oauth_clients
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
