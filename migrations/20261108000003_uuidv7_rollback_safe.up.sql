-- Fix the custom uuidv7() (PostgreSQL < 18) failing for the rest of a
-- connection's life after a rollback.
--
-- The function keeps its monotonicity state in two custom session variables,
-- set with set_config(..., false). If a connection's first uuidv7() call is
-- in a transaction that rolls back, the variables survive the rollback as
-- empty strings. The old code only handled "variable does not exist", so
-- every later call on that connection failed with
--   invalid input syntax for type bigint: ""
-- With a connection pool that means every insert relying on DEFAULT uuidv7()
-- fails whenever it lands on that connection, until the connection closes.
--
-- CREATE OR REPLACE keeps the function's identity, so column defaults that
-- reference it are unaffected.

DO $$
BEGIN
    -- Only where the custom implementation exists (PostgreSQL < 18); the
    -- native pg_catalog.uuidv7() of PostgreSQL 18+ is not affected
    IF EXISTS (
        SELECT 1 FROM pg_proc p
        JOIN pg_namespace n ON p.pronamespace = n.oid
        WHERE p.proname = 'uuidv7'
        AND n.nspname = 'public'
        AND p.pronargs = 0
    ) THEN
        CREATE OR REPLACE FUNCTION public.uuidv7() RETURNS uuid AS $func$
        DECLARE
            ts_ms bigint;
            last_ts bigint;
            counter int;
            rand_bytes bytea;
            uuid_bytes bytea;
        BEGIN
            -- Get current timestamp in milliseconds
            ts_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint;

            -- Get connection-local state from session variables. A variable
            -- that was never set reads as NULL (missing_ok). One whose first
            -- assignment happened in a transaction that rolled back still
            -- exists and reads as an empty string. Both mean "no state yet".
            last_ts := COALESCE(NULLIF(current_setting('uuidv7.last_ts', true), '')::bigint, 0);
            counter := COALESCE(NULLIF(current_setting('uuidv7.counter', true), '')::int, -1);

            IF ts_ms = last_ts THEN
                -- Same millisecond: increment counter
                counter := counter + 1;
                IF counter > 4095 THEN
                    -- Counter overflow: wait for next millisecond
                    -- This is rare (>4096 UUIDs per ms = >4M/sec)
                    WHILE (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint = ts_ms LOOP
                        PERFORM pg_sleep(0.0001);  -- 100μs sleep to avoid busy-wait
                    END LOOP;
                    ts_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint;
                    -- Random start in lower half to leave room for increments
                    counter := (random() * 2047)::int;
                END IF;
            ELSE
                -- New millisecond: reset counter with random start (RFC 9562 recommendation)
                -- Using lower half (0-2047) leaves headroom for ~2048 increments before overflow
                counter := (random() * 2047)::int;
            END IF;

            -- Save state for next call in this connection
            PERFORM set_config('uuidv7.last_ts', ts_ms::text, false);
            PERFORM set_config('uuidv7.counter', counter::text, false);

            -- Get 8 random bytes for rand_b (62 bits, but we get 64 and mask 2)
            rand_bytes := gen_random_bytes(8);

            -- Build 16-byte UUID:
            -- Bytes 0-5: 48-bit timestamp (big-endian)
            -- Byte 6: version (0111 = 7) in high nibble + high 4 bits of counter
            -- Byte 7: low 8 bits of counter
            -- Byte 8: variant (10) in high 2 bits + 6 random bits
            -- Bytes 9-15: 56 random bits

            uuid_bytes :=
                set_byte(set_byte(set_byte(set_byte(set_byte(set_byte(
                    '\x00000000000000000000000000000000'::bytea,
                    0, ((ts_ms >> 40) & 255)::int),
                    1, ((ts_ms >> 32) & 255)::int),
                    2, ((ts_ms >> 24) & 255)::int),
                    3, ((ts_ms >> 16) & 255)::int),
                    4, ((ts_ms >> 8) & 255)::int),
                    5, (ts_ms & 255)::int);

            -- Byte 6: version 7 (0111 = 112) + high 4 bits of 12-bit counter
            uuid_bytes := set_byte(uuid_bytes, 6, (112 | ((counter >> 8) & 15))::int);

            -- Byte 7: low 8 bits of counter
            uuid_bytes := set_byte(uuid_bytes, 7, (counter & 255)::int);

            -- Byte 8: variant (10 = 128) + 6 random bits
            uuid_bytes := set_byte(uuid_bytes, 8, (128 | (get_byte(rand_bytes, 0) & 63))::int);

            -- Bytes 9-15: 56 random bits
            uuid_bytes := set_byte(uuid_bytes, 9, get_byte(rand_bytes, 1));
            uuid_bytes := set_byte(uuid_bytes, 10, get_byte(rand_bytes, 2));
            uuid_bytes := set_byte(uuid_bytes, 11, get_byte(rand_bytes, 3));
            uuid_bytes := set_byte(uuid_bytes, 12, get_byte(rand_bytes, 4));
            uuid_bytes := set_byte(uuid_bytes, 13, get_byte(rand_bytes, 5));
            uuid_bytes := set_byte(uuid_bytes, 14, get_byte(rand_bytes, 6));
            uuid_bytes := set_byte(uuid_bytes, 15, get_byte(rand_bytes, 7));

            RETURN encode(uuid_bytes, 'hex')::uuid;
        END;
        $func$ LANGUAGE plpgsql VOLATILE;
    END IF;
END;
$$;
