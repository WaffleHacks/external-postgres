SELECT
    r.rolname as "username!",
    r.rolcanlogin as "can_login!",
    r.rolcreatedb as "create_db!",
    r.rolcreaterole as "create_role!",
    r.rolbypassrls as "bypass_rls!",
    r.rolsuper as "superuser!"
FROM pg_catalog.pg_roles r
WHERE r.rolname = $1;
