-- "My shop": the system the legacy CRM moves into, for the Move data screenshots and demos.
-- Numeric keys, first and last names, an enum, UTC datetimes, and one customer already in it.
--   docker compose -f dev/docker-compose.yml exec -T mysql mysql -uroot -pkiyi < dev/seed/move-target.sql
DROP DATABASE IF EXISTS kiyi_move;
CREATE DATABASE kiyi_move;
USE kiyi_move;
CREATE TABLE customers (id INT AUTO_INCREMENT PRIMARY KEY, first_name VARCHAR(50) NOT NULL, last_name VARCHAR(50), email VARCHAR(120) NOT NULL UNIQUE, phone VARCHAR(30), status ENUM('active','passive') NOT NULL, is_vip TINYINT(1) NOT NULL DEFAULT 0, created_at DATETIME NOT NULL);
CREATE TABLE products (id INT AUTO_INCREMENT PRIMARY KEY, sku VARCHAR(20) NOT NULL UNIQUE, name VARCHAR(100) NOT NULL, price DECIMAL(10,2) NOT NULL, stock INT NOT NULL DEFAULT 0, attributes JSON, tags JSON);
CREATE TABLE orders (id INT AUTO_INCREMENT PRIMARY KEY, customer_id INT NOT NULL, product_id INT NOT NULL, quantity INT NOT NULL, total DECIMAL(10,2) NOT NULL, ordered_at DATETIME NOT NULL, refunded TINYINT(1) NOT NULL DEFAULT 0,
  FOREIGN KEY (customer_id) REFERENCES customers (id), FOREIGN KEY (product_id) REFERENCES products (id));
INSERT INTO customers (first_name, last_name, email, status, created_at) VALUES ('Selin', 'Arslan', 'selin@myshop.example', 'active', '2023-04-12 10:00:00');
