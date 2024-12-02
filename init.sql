create table if not exists books (
    bookmeter_id bigint primary key,
    amazon_url text not null,
    kindle_id text,
    title text not null,
    basis_price integer,
    price integer,
    discount_rate real,
    updated_at timestamp not null
);
create index if not exists books_kindle_id_index on books (kindle_id);
create index if not exists books_basis_price_index on books (basis_price);
create index if not exists books_price_index on books (price);
create index if not exists books_discount_rate_index on books (discount_rate);
create index if not exists books_title_index on books (title);
