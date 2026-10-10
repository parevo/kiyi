-- "Acme legacy CRM": another company's system, for the Move data screenshots and demos.
-- UUID keys, one-letter status codes, a single name field, time zones.
--   docker compose -f dev/docker-compose.yml exec -T postgres sh -c 'dropdb -U kiyi --if-exists --force kiyi_legacy; createdb -U kiyi kiyi_legacy; psql -q -U kiyi -d kiyi_legacy' < dev/seed/move-legacy.sql
CREATE TABLE clients (client_id uuid PRIMARY KEY, full_name text NOT NULL, e_posta text NOT NULL, phone text, state char(1) NOT NULL, joined timestamptz NOT NULL, vip boolean NOT NULL DEFAULT false);
CREATE TABLE items (code text PRIMARY KEY, title text NOT NULL, unit_price numeric(10,2) NOT NULL, stock int, specs jsonb, tags text[]);
CREATE TABLE purchases (id serial PRIMARY KEY, client uuid NOT NULL REFERENCES clients, item text NOT NULL REFERENCES items, qty int NOT NULL, amount numeric(10,2) NOT NULL, placed timestamptz NOT NULL, status text NOT NULL);

INSERT INTO clients
SELECT md5('c' || g)::uuid,
       (ARRAY['Ada','Zeynep','Noah','Mia','Emre','Lea','Hugo','Priya'])[1 + g % 8] || ' ' || (ARRAY['Lovelace','Yılmaz','Patel','Kim','Demir','Dubois','Costa'])[1 + g % 7],
       lower((ARRAY['ada','zeynep','noah','mia','emre','lea','hugo','priya'])[1 + g % 8]) || g || '@acme-crm.example',
       CASE WHEN g % 3 = 0 THEN NULL ELSE '+90 555 ' || lpad(g::text, 4, '0') END,
       (ARRAY['A','A','P'])[1 + g % 3],
       now() - (g || ' days')::interval - (g * 37 % 1440 || ' minutes')::interval,
       g % 7 = 0
FROM generate_series(1, 250) g;

INSERT INTO items VALUES
  ('LAMP-1', 'Aero Desk Lamp', 89.90, 140, '{"color": "graphite", "watts": 9}', '{office,"warm light"}'),
  ('MUG-2', 'Tide Ceramic Mug', 24.00, NULL, '{"ml": 350}', '{kitchen}'),
  ('BAG-3', 'Northwind Backpack', 129.00, 62, '{"liters": 22}', '{travel,outdoor}');

INSERT INTO purchases (client, item, qty, amount, placed, status)
SELECT md5('c' || (1 + g % 250))::uuid, (ARRAY['LAMP-1','MUG-2','BAG-3'])[1 + g % 3], 1 + g % 3, (ARRAY[89.90, 24.00, 129.00])[1 + g % 3] * (1 + g % 3),
       now() - (g || ' hours')::interval, CASE WHEN g % 9 = 0 THEN 'refunded' ELSE 'done' END
FROM generate_series(1, 1200) g;
