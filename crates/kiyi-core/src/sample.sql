-- "Acme Store": the sample database Kiyi offers on first launch. Statements are separated by
-- blank lines. Values come from arithmetic on the row number, so every copy looks the same.
-- CROSS JOIN keeps the row counter as the outer loop; otherwise SQLite may rerun it per lookup row.

CREATE TABLE customers (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL,
  email          TEXT NOT NULL UNIQUE,
  city           TEXT,
  country        TEXT,
  plan           TEXT NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'starter', 'pro', 'enterprise')),
  active         BOOLEAN NOT NULL DEFAULT 1,
  lifetime_value NUMERIC NOT NULL DEFAULT 0,
  signed_up_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE products (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  sku        TEXT NOT NULL UNIQUE,
  price      NUMERIC NOT NULL,
  in_stock   INTEGER NOT NULL DEFAULT 0,
  details    JSON,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE orders (
  id          INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
  product_id  INTEGER NOT NULL REFERENCES products (id),
  quantity    INTEGER NOT NULL DEFAULT 1,
  total       NUMERIC NOT NULL,
  status      TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid', 'shipped', 'refunded')),
  placed_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX orders_customer_idx ON orders (customer_id);

CREATE INDEX orders_placed_idx ON orders (placed_at);

WITH RECURSIVE
  g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 1200),
  first(i, name) AS (VALUES (0,'Olivia'),(1,'Liam'),(2,'Emma'),(3,'Noah'),(4,'Ava'),(5,'Lucas'),(6,'Mia'),(7,'Leo'),(8,'Sofia'),(9,'Elias'),(10,'Amelia'),(11,'Hugo'),(12,'Isla'),(13,'Mateo'),(14,'Chloe'),(15,'Oscar'),(16,'Lea'),(17,'Felix'),(18,'Nora'),(19,'Arthur'),(20,'Hana'),(21,'Kenji'),(22,'Priya'),(23,'Arjun'),(24,'Zeynep'),(25,'Emre'),(26,'Ines'),(27,'Tomas'),(28,'Freya'),(29,'Malik')),
  last(i, name) AS (VALUES (0,'Smith'),(1,'Garcia'),(2,'Müller'),(3,'Rossi'),(4,'Dubois'),(5,'Kowalski'),(6,'Novak'),(7,'Tanaka'),(8,'Patel'),(9,'Kim'),(10,'Andersen'),(11,'Silva'),(12,'Yılmaz'),(13,'Okafor'),(14,'Jansen'),(15,'Costa'),(16,'Fischer'),(17,'Laurent'),(18,'Nguyen'),(19,'Hughes')),
  places(i, city, country) AS (VALUES (0,'Berlin','Germany'),(1,'Lisbon','Portugal'),(2,'Austin','United States'),(3,'Toronto','Canada'),(4,'Tokyo','Japan'),(5,'Istanbul','Türkiye'),(6,'Paris','France'),(7,'Amsterdam','Netherlands'),(8,'Melbourne','Australia'),(9,'Copenhagen','Denmark'),(10,'Seoul','South Korea'),(11,'São Paulo','Brazil')),
  plans(i, plan) AS (VALUES (0,'free'),(1,'free'),(2,'starter'),(3,'starter'),(4,'pro'),(5,'pro'),(6,'pro'),(7,'enterprise'))
INSERT INTO customers (id, name, email, city, country, plan, active, lifetime_value, signed_up_at)
SELECT g.n,
       f.name || ' ' || l.name,
       lower(replace(f.name, ' ', '') || '.' || replace(replace(l.name, 'ü', 'u'), 'ı', 'i')) || g.n || '@' ||
         CASE g.n % 4 WHEN 0 THEN 'example.com' WHEN 1 THEN 'mail.dev' WHEN 2 THEN 'acme.io' ELSE 'inbox.net' END,
       p.city, p.country, pl.plan,
       (g.n * 7) % 9 <> 0,
       round(((g.n * 7919) % 1000) * ((g.n * 104729) % 1000) / 111.0, 2),
       datetime('now', '-' || ((g.n * 7907) % 900) || ' days', '-' || ((g.n * 61) % 1440) || ' minutes')
FROM g
CROSS JOIN first f ON f.i = (g.n * 7) % 30
CROSS JOIN last l ON l.i = (g.n * 13) % 20
CROSS JOIN places p ON p.i = (g.n * 5) % 12
CROSS JOIN plans pl ON pl.i = (g.n * 11) % 8;

INSERT INTO products (id, name, sku, price, in_stock, details) VALUES
  (1, 'Aero Desk Lamp', 'LAMP-001', 89.00, 140, '{"color": "graphite", "watts": 9}'),
  (2, 'Northwind Backpack', 'BAG-014', 129.00, 62, '{"liters": 22, "waterproof": true}'),
  (3, 'Tide Ceramic Mug', 'MUG-003', 24.00, 510, '{"ml": 350, "dishwasher": true}'),
  (4, 'Harbor Wool Throw', 'HOME-221', 149.00, 35, '{"size": "130x170", "material": "merino"}'),
  (5, 'Drift Wireless Charger', 'TECH-090', 59.00, 220, '{"watts": 15}'),
  (6, 'Coastline Notebook', 'PAPER-007', 18.00, 900, '{"pages": 192, "grid": "dot"}'),
  (7, 'Pebble Speaker', 'TECH-112', 199.00, 48, '{"battery_h": 20}'),
  (8, 'Sandbar Sunglasses', 'ACC-031', 119.00, 77, '{"polarized": true}');

WITH RECURSIVE
  g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g WHERE n < 9000),
  statuses(i, status) AS (VALUES (0,'paid'),(1,'paid'),(2,'paid'),(3,'shipped'),(4,'shipped'),(5,'pending'),(6,'refunded'))
INSERT INTO orders (customer_id, product_id, quantity, total, status, placed_at)
SELECT 1 + (g.n * 7919) % 1200,
       pr.id,
       1 + g.n % 3,
       round(pr.price * (1 + g.n % 3), 2),
       s.status,
       datetime('now', '-' || ((g.n * 7907) % 400) || ' days', '-' || ((g.n * 37) % 1440) || ' minutes')
FROM g
CROSS JOIN products pr ON pr.id = 1 + (g.n * 31) % 8
CROSS JOIN statuses s ON s.i = (g.n * 13) % 7;

CREATE VIEW customer_totals AS
SELECT c.id, c.name, c.country, count(o.id) AS orders, coalesce(sum(o.total), 0) AS spent
FROM customers c LEFT JOIN orders o ON o.customer_id = c.id AND o.status <> 'refunded'
GROUP BY c.id;

CREATE TRIGGER products_updated_at
AFTER UPDATE ON products
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE products SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;
