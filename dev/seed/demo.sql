-- "Acme Store": realistic-looking data for screenshots and demos.
--   docker compose -f dev/docker-compose.yml exec -T postgres sh -c 'createdb -U kiyi demo; psql -q -U kiyi -d demo' < dev/seed/demo.sql
CREATE TYPE plan AS ENUM ('free', 'starter', 'pro', 'enterprise');
CREATE TYPE order_status AS ENUM ('pending', 'paid', 'shipped', 'refunded');

CREATE TABLE customers (
  id            bigserial PRIMARY KEY,
  name          text NOT NULL,
  email         text NOT NULL UNIQUE,
  city          text,
  country       text,
  plan          plan NOT NULL DEFAULT 'free',
  active        boolean NOT NULL DEFAULT true,
  lifetime_value numeric(12, 2) NOT NULL DEFAULT 0,
  signed_up_at  timestamptz NOT NULL DEFAULT now()
);
COMMENT ON COLUMN customers.lifetime_value IS 'Total paid, in USD';

CREATE TABLE products (
  id        bigserial PRIMARY KEY,
  name      text NOT NULL,
  sku       text NOT NULL UNIQUE,
  price     numeric(10, 2) NOT NULL,
  in_stock  integer NOT NULL DEFAULT 0,
  details   jsonb
);

CREATE TABLE orders (
  id           bigserial PRIMARY KEY,
  customer_id  bigint NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
  product_id   bigint NOT NULL REFERENCES products (id),
  quantity     integer NOT NULL DEFAULT 1,
  total        numeric(12, 2) NOT NULL,
  status       order_status NOT NULL DEFAULT 'pending',
  placed_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX orders_customer_idx ON orders (customer_id);
CREATE INDEX orders_placed_idx ON orders (placed_at);

-- Deterministic pseudo-randomness so screenshots are stable between runs.
SELECT setseed(0.42);

WITH first(n) AS (SELECT unnest(ARRAY['Olivia','Liam','Emma','Noah','Ava','Lucas','Mia','Leo','Sofia','Elias','Amelia','Hugo','Isla','Mateo','Chloe','Oscar','Lea','Felix','Nora','Arthur','Hana','Kenji','Priya','Arjun','Zeynep','Emre','Ines','Tomás','Freya','Malik'])),
     last(n)  AS (SELECT unnest(ARRAY['Smith','García','Müller','Rossi','Dubois','Kowalski','Novak','Tanaka','Patel','Kim','Andersen','Silva','Yılmaz','O''Brien','Jansen','Costa','Fischer','Laurent','Nguyen','Hughes'])),
     places(city, country) AS (VALUES ('Berlin','Germany'),('Lisbon','Portugal'),('Austin','United States'),('Toronto','Canada'),('Tokyo','Japan'),('Istanbul','Türkiye'),('Paris','France'),('Amsterdam','Netherlands'),('Melbourne','Australia'),('Copenhagen','Denmark'),('Seoul','South Korea'),('São Paulo','Brazil'))
INSERT INTO customers (name, email, city, country, plan, active, lifetime_value, signed_up_at)
SELECT f.n || ' ' || l.n,
       lower(translate(f.n || '.' || replace(l.n, '''', ''), 'áéíóúüöçışğñã', 'aeiouuocisgna')) || g || '@' || (ARRAY['example.com','mail.dev','acme.io','inbox.net'])[1 + g % 4],
       p.city, p.country,
       (ARRAY['free','free','starter','starter','pro','pro','pro','enterprise']::plan[])[1 + floor(random() * 8)::int],
       random() > 0.12,
       round((random() * random() * 9000)::numeric, 2),
       now() - (random() * interval '900 days')
FROM generate_series(1, 2400) g
CROSS JOIN LATERAL (SELECT n FROM first ORDER BY random() + g * 0 LIMIT 1) f
CROSS JOIN LATERAL (SELECT n FROM last ORDER BY random() + g * 0 LIMIT 1) l
CROSS JOIN LATERAL (SELECT city, country FROM places ORDER BY random() + g * 0 LIMIT 1) p;

INSERT INTO products (name, sku, price, in_stock, details) VALUES
  ('Aero Desk Lamp', 'LAMP-001', 89.00, 140, '{"color": "graphite", "watts": 9}'),
  ('Northwind Backpack', 'BAG-014', 129.00, 62, '{"liters": 22, "waterproof": true}'),
  ('Tide Ceramic Mug', 'MUG-003', 24.00, 510, '{"ml": 350, "dishwasher": true}'),
  ('Harbor Wool Throw', 'HOME-221', 149.00, 35, '{"size": "130x170", "material": "merino"}'),
  ('Drift Wireless Charger', 'TECH-090', 59.00, 220, '{"watts": 15}'),
  ('Coastline Notebook', 'PAPER-007', 18.00, 900, '{"pages": 192, "grid": "dot"}'),
  ('Pebble Speaker', 'TECH-112', 199.00, 48, '{"battery_h": 20}'),
  ('Sandbar Sunglasses', 'ACC-031', 119.00, 77, '{"polarized": true}');

INSERT INTO orders (customer_id, product_id, quantity, total, status, placed_at)
SELECT c, p, q, round((pr.price * q)::numeric, 2),
       (ARRAY['paid','paid','paid','shipped','shipped','pending','refunded']::order_status[])[1 + floor(random() * 7)::int],
       now() - (random() * interval '400 days')
FROM (SELECT 1 + floor(random() * 2400)::int AS c, 1 + floor(random() * 8)::int AS p, 1 + floor(random() * 3)::int AS q FROM generate_series(1, 18000)) x
JOIN products pr ON pr.id = x.p;

ANALYZE;
