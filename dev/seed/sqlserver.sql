-- SQL Server test data, like the PostgreSQL and MySQL seeds. Run once the server is up:
--   docker compose -f dev/docker-compose.yml exec sqlserver /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P Kiyi_pass1 -i /seed/sqlserver.sql
IF DB_ID('kiyi_test') IS NOT NULL
BEGIN
  ALTER DATABASE kiyi_test SET SINGLE_USER WITH ROLLBACK IMMEDIATE;
  DROP DATABASE kiyi_test;
END
GO
CREATE DATABASE kiyi_test;
GO
USE kiyi_test;
GO
CREATE TABLE customers (
  id int IDENTITY(1,1) PRIMARY KEY,
  public_id uniqueidentifier NOT NULL DEFAULT NEWID(),
  email nvarchar(255) NOT NULL UNIQUE,
  full_name nvarchar(120) NULL,
  is_active bit NOT NULL DEFAULT 1,
  balance decimal(30,10) NOT NULL DEFAULT 0,
  avatar varbinary(max) NULL,
  created_at datetime2 NOT NULL DEFAULT SYSDATETIME()
);
CREATE TABLE orders (
  id bigint IDENTITY(1,1) PRIMARY KEY,
  customer_id int NOT NULL CONSTRAINT fk_orders_customer REFERENCES customers(id),
  status nvarchar(20) NOT NULL DEFAULT 'pending',
  total decimal(12,2) NOT NULL,
  placed_on date NOT NULL DEFAULT CAST(GETDATE() AS date),
  note nvarchar(200) NULL
);
CREATE INDEX orders_customer_idx ON orders (customer_id);
GO
INSERT INTO customers (email, full_name, is_active, balance, avatar) VALUES
  (N'ayse@example.com', N'Ayşe Yılmaz', 1, 12345678901234567890.0123456789, 0xDEADBEEF),
  (N'mehmet@example.com', NULL, 0, 0, NULL);
INSERT INTO customers (email, full_name)
SELECT CONCAT(N'user', n, N'@example.com'), CONCAT(N'Kullanıcı ', n)
FROM (SELECT TOP 1000 ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS n FROM sys.all_objects a CROSS JOIN sys.all_objects b) x;
INSERT INTO orders (customer_id, status, total, placed_on, note)
SELECT 1 + (n % 1000), CASE n % 3 WHEN 0 THEN N'pending' WHEN 1 THEN N'paid' ELSE N'shipped' END, (n % 500) + 0.99,
  DATEADD(day, -(n % 90), CAST('2026-10-09' AS date)), CASE WHEN n % 10 = 0 THEN N'50% indirim [kampanya]' END
FROM (SELECT TOP 2000 ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS n FROM sys.all_objects a CROSS JOIN sys.all_objects b) x;
GO
CREATE VIEW active_customers AS SELECT id, email FROM customers WHERE is_active = 1;
GO
