DROP TABLE IF EXISTS order_items, orders, products, customers CASCADE;

CREATE TABLE customers (
  id serial PRIMARY KEY,
  name text NOT NULL,
  email text NOT NULL,
  country text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE products (
  id serial PRIMARY KEY,
  sku text NOT NULL,
  title text NOT NULL,
  price numeric(8,2) NOT NULL,
  stock int NOT NULL
);
CREATE TABLE orders (
  id serial PRIMARY KEY,
  customer_id int NOT NULL REFERENCES customers(id),
  status text NOT NULL,
  total numeric(10,2) NOT NULL,
  placed_at timestamptz NOT NULL
);
CREATE TABLE order_items (
  id serial PRIMARY KEY,
  order_id int NOT NULL REFERENCES orders(id),
  product_id int NOT NULL REFERENCES products(id),
  qty int NOT NULL
);

INSERT INTO customers (name, email, country, created_at) VALUES
 ('Ada Lovelace','ada@analytical.io','UK', now() - interval '400 days'),
 ('Grace Hopper','grace@navy.mil','US', now() - interval '380 days'),
 ('Linus Torvalds','linus@kernel.org','FI', now() - interval '300 days'),
 ('Margaret Hamilton','margaret@apollo.space','US', now() - interval '250 days'),
 ('Alan Turing','alan@bletchley.uk','UK', now() - interval '200 days'),
 ('Katherine Johnson','katherine@nasa.gov','US', now() - interval '150 days'),
 ('Dennis Ritchie','dmr@bell-labs.com','US', now() - interval '120 days'),
 ('Barbara Liskov','barbara@mit.edu','US', now() - interval '90 days'),
 ('Yukihiro Matsumoto','matz@ruby-lang.org','JP', now() - interval '60 days'),
 ('Hedy Lamarr','hedy@spread.spectrum','AT', now() - interval '30 days');

INSERT INTO products (sku, title, price, stock) VALUES
 ('KB-001','Mechanical Keyboard',129.00,42),
 ('MS-002','Ergonomic Mouse',59.50,120),
 ('MN-003','4K Monitor 27"',429.99,15),
 ('HD-004','Noise-cancelling Headphones',249.00,60),
 ('DK-005','Standing Desk',599.00,8),
 ('LP-006','Laptop Stand',39.90,200);

INSERT INTO orders (customer_id, status, total, placed_at)
SELECT 1 + (g % 10),
       (ARRAY['paid','shipped','delivered','refunded','pending'])[1 + (g % 5)],
       round((20 + random()*900)::numeric, 2),
       now() - (g || ' hours')::interval * 7
FROM generate_series(1, 400) g;

INSERT INTO order_items (order_id, product_id, qty)
SELECT o.id, 1 + (o.id % 6), 1 + (o.id % 3) FROM orders o;
CREATE INDEX ON orders (customer_id);
ANALYZE;
