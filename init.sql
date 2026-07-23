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
alter table books add column if not exists active_at timestamp default null;
alter table books add column if not exists is_kindle_unlimited boolean not null default false;
alter table books add column if not exists binding_name text;

-- 中古本サイト (bookoff / valuebooks / netoff) の商品オファー
create table if not exists used_book_offers (
    bookmeter_id bigint not null references books(bookmeter_id) on delete cascade,
    site text not null,
    product_id text,
    product_url text,
    price integer,
    condition text,
    in_stock boolean not null default false,
    updated_at timestamp not null,
    primary key (bookmeter_id, site)
);
create index if not exists used_book_offers_product_id_index on used_book_offers (product_id);
