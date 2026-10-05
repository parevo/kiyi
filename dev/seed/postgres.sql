CREATE TYPE order_status AS ENUM ('pending', 'paid', 'shipped');

CREATE TABLE customers (
  id          bigserial PRIMARY KEY,
  public_id   uuid NOT NULL DEFAULT gen_random_uuid(),
  email       text NOT NULL UNIQUE,
  full_name   varchar(120),
  is_active   boolean NOT NULL DEFAULT true,
  balance     numeric(30, 10) NOT NULL DEFAULT 0,
  tags        text[],
  profile     jsonb,
  avatar      bytea,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE orders (
  id          bigserial PRIMARY KEY,
  customer_id bigint NOT NULL REFERENCES customers(id),
  status      order_status NOT NULL DEFAULT 'pending',
  total       numeric(12, 2) NOT NULL,
  placed_on   date NOT NULL DEFAULT current_date
);

CREATE VIEW active_customers AS SELECT id, email FROM customers WHERE is_active;

INSERT INTO customers (email, full_name, is_active, balance, tags, profile, avatar)
VALUES
  ('ayse@example.com', 'Ayşe Yılmaz', true, 12345678901234567890.0123456789, '{vip,early}', '{"plan": "pro", "seats": 5}', '\xdeadbeef'),
  ('mehmet@example.com', NULL, false, 0, NULL, NULL, NULL);

INSERT INTO customers (email, full_name, balance)
SELECT 'user' || g || '@example.com', 'Kullanıcı ' || g, g * 1.5 FROM generate_series(1, 20000) g;

INSERT INTO orders (customer_id, status, total)
SELECT 1 + (g % 1000), (ARRAY['pending', 'paid', 'shipped'])[1 + g % 3]::order_status, (g % 500) + 0.99
FROM generate_series(1, 50000) g;

CREATE INDEX orders_customer_id_idx ON orders (customer_id);
COMMENT ON COLUMN customers.email IS 'Giriş e-postası';
