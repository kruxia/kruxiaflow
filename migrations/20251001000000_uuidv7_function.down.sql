-- Drop the custom uuidv7 function (only if it exists in public schema)
-- Does not affect native uuidv7() in PostgreSQL 18+ (which is in pg_catalog)
DROP FUNCTION IF EXISTS public.uuidv7();
