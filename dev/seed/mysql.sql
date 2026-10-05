SET NAMES utf8mb4;

CREATE TABLE customers (
  id          BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  public_id   BINARY(16) NOT NULL DEFAULT (UUID_TO_BIN(UUID())),
  email       VARCHAR(190) NOT NULL UNIQUE,
  full_name   VARCHAR(120),
  is_active   TINYINT(1) NOT NULL DEFAULT 1,
  balance     DECIMAL(30, 10) NOT NULL DEFAULT 0,
  profile     JSON,
  status      ENUM('pending', 'paid', 'shipped') NOT NULL DEFAULT 'pending',
  created_at  DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) DEFAULT CHARSET = utf8mb4;

CREATE VIEW active_customers AS SELECT id, email FROM customers WHERE is_active = 1;

INSERT INTO customers (email, full_name, is_active, balance, profile)
VALUES
  ('ayse@example.com', 'Ayşe Yılmaz', 1, 12345678901234567890.0123456789, '{"plan": "pro", "seats": 5}'),
  ('mehmet@example.com', NULL, 0, 0, NULL);

CREATE TABLE orders (
  id          BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  customer_id BIGINT UNSIGNED NOT NULL,
  total       DECIMAL(12, 2) NOT NULL,
  note        VARCHAR(200) COMMENT 'Müşteri notu',
  updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  KEY orders_customer_idx (customer_id),
  CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES customers (id) ON DELETE CASCADE
) DEFAULT CHARSET = utf8mb4;

INSERT INTO orders (customer_id, total, note) VALUES (1, 99.90, 'ilk sipariş'), (1, 15.00, NULL), (2, 42.42, 'hediye');
