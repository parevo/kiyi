-- 1,000,000 rows of mixed types for dev/bench.
SET SESSION cte_max_recursion_depth = 1000001;
DROP TABLE IF EXISTS events;
CREATE TABLE events (
  id          bigint PRIMARY KEY,
  user_id     int NOT NULL,
  kind        varchar(16) NOT NULL,
  amount      decimal(12,2),
  ok          boolean NOT NULL,
  created_at  datetime NOT NULL,
  note        text,
  payload     json,
  KEY events_created_at (created_at)
);
INSERT INTO events
WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 1000000)
SELECT n,
       (n * 7919) % 50000,
       ELT(1 + n % 5, 'view','click','signup','purchase','refund'),
       ROUND(((n * 104729) % 100000) / 100.0, 2),
       n % 3 <> 0,
       TIMESTAMP '2026-01-01 00:00:00' + INTERVAL n SECOND,
       IF(n % 4 = 0, NULL, CONCAT('note number ', n)),
       JSON_OBJECT('page', CONCAT('/p/', n % 120), 'ms', n % 900)
FROM g;
ANALYZE TABLE events;
