-- Drop OAuth authentication tables

DROP TRIGGER IF EXISTS update_oauth_clients_updated_at ON oauth_clients;
DROP TRIGGER IF EXISTS update_oauth_users_updated_at ON oauth_users;
DROP FUNCTION IF EXISTS update_updated_at_column();

DROP TABLE IF EXISTS oauth_refresh_tokens;
DROP TABLE IF EXISTS oauth_clients;
DROP TABLE IF EXISTS oauth_users;
