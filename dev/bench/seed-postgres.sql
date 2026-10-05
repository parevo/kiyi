-- 1,000,000 rows of mixed types for dev/bench. Separate database so tests and demos are untouched.
DROP TABLE IF EXISTS events;
CREATE TABLE events (
  id          bigint PRIMARY KEY,
  user_id     integer NOT NULL,
  kind        text NOT NULL,
  amount      numeric(12,2),
  ok          boolean NOT NULL,
  created_at  timestamptz NOT NULL,
  note        text,
  payload     jsonb
);
INSERT INTO events
SELECT g,
       ((g::bigint * 7919) % 50000)::int,
       (ARRAY['view','click','signup','purchase','refund'])[1 + g % 5],
       round(((g::bigint * 104729) % 100000) / 100.0, 2),
       g % 3 <> 0,
       timestamptz '2026-01-01' + (g || ' seconds')::interval,
       CASE WHEN g % 4 = 0 THEN NULL ELSE 'note number ' || g END,
       jsonb_build_object('page', '/p/' || (g % 120), 'ms', g % 900)
FROM generate_series(1, 1000000) g;
CREATE INDEX events_created_at ON events (created_at);
ANALYZE events;
