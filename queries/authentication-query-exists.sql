SELECT p.oid FROM pg_catalog.pg_proc p
    INNER JOIN pg_catalog.pg_namespace n
        ON p.pronamespace = n.oid
    WHERE p.proname = 'user_lookup' AND n.nspname = 'pgbouncer';
